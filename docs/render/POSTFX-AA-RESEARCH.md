# Post-processing and anti-aliasing — the survey

> **Status:** architect's research record, 2026-09-25. It covers trunk **`6394bc5e`** (branch
> `integ/unified`), read through the worktree `D:/wt/docs`.
>
> **How it was made.** It folds together three passes. The first is a researcher report, relayed
> verbatim. The second is the architect's own re-reading of the tree and re-fetching of the
> sources the design leans on. The third (the same day) revises both documents after the
> architecture critique of the design; it re-read the tree at the critique's sites and fetched the
> primary sources the critique pointed to. §6 lists every correction: C1–C12 from the second pass,
> C13–C27 from the third.
>
> **What was not done.** No `cargo` command, build, test, timing or GPU run was taken, because
> three build lanes were running on the machine. **No number in this document was measured
> here.**
>
> **Companion document.** The design built on this survey is
> [`POSTFX-AA-DESIGN-SPACE.md`](POSTFX-AA-DESIGN-SPACE.md). It cites this document by section,
> technique number (**T1–T31**) and pitfall number (**P1–P29**).
>
> **Citations and gates.** Code is cited as a path plus a symbol. A `path:line` appears only
> where the line was re-opened at `6394bc5e`. This directory is not in `GATED_DOCS`
> (`tests/internal_docs_anchors.rs`), so nothing machine-checks these citations.

## Confidence tags

| Tag | Meaning |
|---|---|
| **[T]** | Checked in the tree at `6394bc5e`. |
| **[F]** | A source this pass fetched and read itself. |
| **[R]** | A source the researcher fetched, relayed and not re-fetched here. |
| **[S]** | Seen only through a search-index snippet, never fetched. Weak evidence. |
| **[M]** | A third party's measurement. |
| **[V]** | A vendor's claim about its own product. |
| **[E]** | An estimate or derivation. Its working is shown. |

The WebSearch budget for the session was exhausted before the second pass began, and it stayed
exhausted in the third. Every source added by either pass was fetched from a URL already known. Four PDFs could not be parsed: the DLSS Programming
Guide, Garcia 2017, Lauritzen 2010 and Keinert 2014. The Frostbite course notes exceed the
fetcher's 10 MB limit. Nothing is reported from any of them from memory. Where a fact depends on
one of them, the text says "not verified".

---

## 0. What the tree holds today

### 0.1 The colour pipeline

| Item | Fact [T] | Where |
|---|---|---|
| Scene colour | `lit` is `R8G8B8A8_UNORM`, one image per frame in flight. It is a storage image on every compute producer and a colour attachment on Forward. | `crates/boyko_rhi_vulkan/src/present/targets.rs`: `GBufferTargets::lit` (`:127`) and `GBUFFER_FORMAT` (`:2279`) |
| Where tonemap happens | Tonemap and OETF run **inside the producers**. Eight shaders include the tail: `deferred_pbr.hlsl`, `forward_opaque.fs.hlsl`, `forward_sky.fs.hlsl`, `sdf_forward_march.comp.hlsl`, `ssaa_downsample.fs.hlsl`, `vb_resolve.comp.hlsl`, `vb_shade.comp.hlsl`, `vb_shade_split.comp.hlsl`. | Found by grepping `shaders/` for `OETF_GAMMA_EXP\|tonemap_select` |
| OETF | A hand-written `pow(x, 1/2.2)` (`pbr_lighting.hlsli:202`), not the piecewise sRGB curve. The swapchain prefers `*_UNORM` in `SRGB_NONLINEAR`. The host mirror is `goldens.rs` `tonemap_and_oetf` (`crates/boyko_rhi_vulkan/src/goldens.rs:1917`). | `crates/boyko_rhi_vulkan/src/present/surface.rs` `pick_surface_format` |
| Tonemap curves | `Tonemapper::{Aces (Hill fit, the default), Neutral (Khronos PBR Neutral), ReinhardJodie}`, packed into light-header word 7 at bits 8..11. All three are hand-written HLSL (`aces_fitted`, `khronos_pbr_neutral`, `reinhard_jodie`, `tonemap_select` at `pbr_lighting.hlsli:237`). The eDSL holds no tonemap code. | `crates/boyko_render/src/light.rs` `Tonemapper`, `TONEMAP_MODE_SHIFT` |
| Exposure | One scalar, "the FINAL multiply on accumulated linear radiance" (`light.rs:564`). The shaders read it as `H.exposure`. `particle_draw.fs.hlsl` does not apply it. | `LightingConfig::exposure` |
| Light units | `DirectionalLight::illuminance` is documented as "Illuminance in lux (physical)" (`light.rs:316`), but the test scenes light their suns at 3.1 and 2.8 (`crates/boyko_app/tests/forward_mesh.rs:125`, `crates/boyko_app/tests/csm_fit_eval.rs:74`). `sky_diffuse` and `sky_spec` are legacy unitless constants. The radiance scale is therefore display-referred: roughly 2¹⁵ below physical daylight [E; daylight's ~10⁵ lux is recalled]. | `crates/boyko_render/src/light.rs` `DirectionalLight::new` (`:1216`) |
| HDR formats in the RHI | `Format::B10G11R11UfloatPack32` and `R16G16B16A16Sfloat` exist. B10G11R11 storage support is **optional**. It is probed only for the DDGI atlas (`ddgi_irr_storage_ok`). | `crates/boyko_rhi/src/enums.rs`, `crates/boyko_rhi_vulkan/src/device.rs` |
| HDR swapchain | No `VK_EXT_swapchain_colorspace` and no `VK_EXT_hdr_metadata`. The only colour-space constant is `VK_COLOR_SPACE_SRGB_NONLINEAR_KHR`. | `crates/boyko_rhi_vulkan/src/ffi.rs`, `present/surface.rs` |

### 0.2 Anti-aliasing and sharpening

| Item | Fact [T] | Where |
|---|---|---|
| AA modes | `AaMode::{Off, Fxaa, Smaa, Ssaa, Taa}`, **mutually exclusive**. FXAA, SMAA and the SSAA downsample are fragment passes. TAA and RCAS are compute. | `crates/boyko_render/src/aa_config.rs` `AaMode`; `shaders/{fxaa.fs, smaa_edge.fs, smaa_weight.fs, smaa_blend.fs, ssaa_downsample.fs, taa_resolve.comp, rcas.comp}.hlsl` |
| AA output | `aa_out` is `R8G8B8A8_UNORM`, one per frame in flight (`targets.rs:428`). It is **not a framegraph resource**: `record_taa` hand-records its barriers. This is stated in the TAA pass declaration comment in `graph_bridge.rs`. | `present/graph_bridge.rs` `declare_deferred_graph` |
| Changing AA mode | A different `AaArm` forces the same fence-safe target rebuild an extent change does. | `targets.rs` `GBufferTargets::aa_arm` |
| TAA defaults | Halton(2,3) with 8 taps. `JitterScope::RasterAndBasis` (`taa_config.rs:467`), so the SDF ray basis is sheared too. Variance clip in RGB, γ = 1.0, clipped toward the centre. 16-tap Catmull-Rom history read with `Load`s. Luma weighting on. Blend 0.1 falling to 0.015. Camera-only motion vectors. Only off-screen samples count as disoccluded. | `crates/boyko_render/src/taa_config.rs` `TaaConfig::default`; `taa_resolve.comp.hlsl` header |
| NaN and range handling | **None in the resolve.** A grep of `taa_resolve.comp.hlsl` for `isnan\|isfinite\|64.0` finds nothing, and the output is written raw (`:519`). The protection is implicit: every producer stores into RGBA8 UNORM, whose float → UNORM conversion maps NaN to 0 (as the design critique states it; the spec text was not fetched). The resolve's comment depends on it: "No NaN risk: `cur_lit` is the resolve's own finite LDR output" (`:410`). A NaN that did reach the history would pass through the Catmull-Rom sum (0 × NaN = NaN) and through `clip_toward_aabb_center`, whose `ma_unit > 1.0` test is false for NaN. HLSL `min`/`max` lower to `NMin`/`NMax`, which select the non-NaN operand ([`../VB-SV0-SDF-SHADOW-PLAN.md`](../VB-SV0-SDF-SHADOW-PLAN.md)), but this path goes through additions. | `sample_history_catmull_rom`, `clip_toward_aabb_center` |
| Camera cuts | `TaaState` resets the history only on TAA's first armed frame and on a resize (its doc; the `mark_reset` call in `crates/boyko_app/src/runner.rs`). A change of active camera does not reset it. | `crates/boyko_render/src/taa_state.rs` |
| TAA history | `taa_hist`: 2× `R16G16B16A16_SFLOAT` (`targets.rs:480`). Frame `fi` writes slot `[fi]` and reads `[1-fi]`. | `GBufferTargets::taa_hist` |
| TAA placement | `taa_resolve` is declared after `particle_draw` and before `present_sample` (`graph_bridge.rs:2527`, the particle draw at `:598`). It reads `lit` at `SHADER_READ_ONLY_OPTIMAL`, `viewt` and `taa_hist_read`, and writes `taa_hist`. | `declare_deferred_graph`, `declare_vb_graph` |
| Sharpening | The sharpen ping-pongs through `taa_resolved` (RGBA8, storage only, `targets.rs:515`) into `aa_out`, and runs only when TAA is armed and `SharpenMode::Rcas` is selected (the default is `None`). **The kernel is AMD CAS, not FSR 1 RCAS**: a 3×3 neighbourhood of 9 taps, amplitude `sqrt(saturate(min(mn, 2 − mx)/mx))` (`rcas.comp.hlsl:99`), peak lerped from −1/8 to −1/5 (`:105`), on RGBA8 display values. | `present/passes/rcas.rs`, `shaders/rcas.comp.hlsl` |
| Which paths support AA | `taa_supported()` and `post_process_aa_supported()` are true only for Deferred and VisibilityBuffer (`render_path_config.rs:676`, `:703`). Forward and Forward+ have **no AA seam at all**. | `crates/boyko_render/src/render_path_config.rs` |
| MSAA | None. `rasterization_samples: VK_SAMPLE_COUNT_1_BIT` is hard-coded (`rhi_impl/device.rs:1865`). | `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` |
| Texture mip bias | None: `mip_lod_bias: 0.0` (`bindless.rs:185`). VB shading samples with `SampleGrad` and analytic gradients (`vb_shade.comp.hlsl:367`). The raster G-buffer uses implicit `Sample` (`gbuffer_mrt.fs.hlsl:230`). | `crates/boyko_rhi_vulkan/src/bindless.rs` |

### 0.3 Depth and motion inputs

| Item | Fact [T] | Where |
|---|---|---|
| Depth proxy | `gViewT` is `R32_SFLOAT` ray `t` (not view z), **one per frame in flight** (`targets.rs:133`). So `viewt[1-fi]`, the previous frame's value, still exists while frame `fi` runs. Forward's graph declares no `viewt` write for the SDF march, per an invariant in `declare_forward_graph`. | `GBufferTargets::viewt` |
| Motion vectors | Camera-only reprojection through `MotionCamState` and `gViewT`. A mesh motion-vector MRT and SDF motion vectors exist only under `hwrt` (`targets.rs:181`, `motion_vec`). `MvSource::PerObject` was **investigated and declined**; its doc gives the reasons. The previous-transform carry is the dense `PrevInstanceModelCol` (`crates/boyko_render/src/instance_model.rs`). | `crates/boyko_render/src/motion_cam.rs`, `taa_config.rs` `MvSource` |
| Previous-depth rejection | `DisocclusionTest::OffScreenAndDepth` is declared but **inert**: its doc says the resolve "retains no previous-frame depth". | `taa_config.rs` `DisocclusionTest` |

### 0.4 RHI and frame graph

| Item | Fact [T] | Where |
|---|---|---|
| Queues | One GRAPHICS\|COMPUTE queue, chosen by `find_queue_family`. No async compute, no timeline semaphores, `synchronization2` off. | `crates/boyko_rhi_vulkan/src/device.rs` |
| Subgroups | Only BASIC and BALLOT are required (`REQUIRED_SUBGROUP_OPERATIONS`, `device.rs:3043`), in the compute stage only. | `device.rs` |
| Other device features | `shaderFloat16` is not enabled. `shaderStorageImageWriteWithoutFormat` is not enabled. Every storage writer pins its format with `[[vk::image_format]]`, e.g. `taa_resolve.comp.hlsl:104`. | `device.rs`; the shaders |
| Indirect draws | `vkCmdDispatchIndirect` is loaded, and particles call the raw function directly. **The RHI trait's `RhiCommandEncoder::dispatch_indirect` is a silent no-op** (`crates/boyko_rhi/src/encoder.rs:408`, a `#[cold]` default body), and the Vulkan backend does not override it. The same-day reflections update books the override as RK-16 (its U7). There is no `DrawIndexedIndirectCount`. | `device.rs` `cmd_dispatch_indirect`; `present/passes/particles.rs` |
| Specialization constants | Supported: `rhi_impl/device.rs:821` builds a `VkSpecializationInfo` at pipeline creation, and the `spec_constant_smoke` cargo feature smoke-tests it on a device. | `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` |
| 3D images | `TextureDimension::D3` exists. The brick atlas is a 3D image with `TRANSFER_DST \| SAMPLED` usage. **No 3D storage image exists yet**: `enums.rs` calls it "the D3 storage image of a later rung". | `crates/boyko_rhi_vulkan/src/brick_atlas.rs` |
| Mip chains | `FrameGraph::add_image_mipped` (`framegraph/graph.rs:392`) tracks each mip level. Its seed is a required argument. The one consumer is the VB `hzb_build`. | `framegraph/graph.rs` |
| Cross-frame seeds | `ResSync::seeded_readers`, `seeded_readers_at_layout` (`framegraph/sync.rs:271`) and `seeded_writer`. The last is documented on "the cluster `alloc` counter" precedent: a sibling frame's undrained write. | `framegraph/sync.rs` |
| Graph rebuild | The frame graph is **re-declared every frame** (`frame_driver.rs:896`, "Zero-alloc (`reset` retains capacity)"). A pass gated at declare time costs nothing on frames where it is off. | `crates/boyko_rhi_vulkan/src/present/frame_driver.rs` |
| Fixed image slots | The Deferred graph's image ResIds come in a **fixed order**: 16 slots, or 22 under `hwrt`, and new images are appended (`FRAMEGRAPH_IMAGE_COUNT`, `graph_bridge.rs:765` and `:770`). | `present/graph_bridge.rs` |
| No transient aliasing | The public render-graph API (`ExtensionPoint`, `TransientImagePool`) is plan-only; grepping `crates/` finds none of its symbols. | [`../RENDER-GRAPH-API-PLAN.md`](../RENDER-GRAPH-API-PLAN.md) |
| GPU timestamps | Zone families `ZONE_BASE_VB`, `ZONE_BASE_GBUFFER`, `ZONE_BASE_SV0` and `ZONE_BASE_PARTICLE` (`gpu_zone.rs:139` is the last), each 16 wide. **No zone covers any AA or post pass.** The Deferred fine marcher has `ZONE_SV0_MARCHER`; Forward's `sdf_forward_march` has no zone (`present/passes/forward.rs` records none). | `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` |

### 0.5 Cameras, the eDSL and the SDF marcher

| Item | Fact [T] | Where |
|---|---|---|
| Cameras | Any number of `Camera` entities may exist, but **one** is rendered per frame: `ActiveCamera` overrides, otherwise the highest `order` among active cameras wins. | `crates/boyko_scene/src/camera.rs` `ActiveCamera`, `resolve_active_camera` |
| eDSL scalar operations | `FieldScalar` has add, sub, mul, div, min, max, clamp01, lerp, abs, sqrt, select and comparisons. `Cf` adds sin, cos and rsqrt. There is **no `log2`, `exp2` or `pow`**. The transcendentals are on `Cf` **by design**: `Cf::sin`'s doc places them "on the control-flow axis, whose Eval instantiation is a codegen-only ZST no physics-reachable code calls", not on `FieldScalar`, "whose `f32` impl IS the physics leaf". `EvalCf::sin` uses `core::intrinsics::sinf32` under the `nightly` feature and `f32::sin` (linking `std`) otherwise. `InterpBackend` (`interp.rs`, gated on `feature = "emit"`) is the same firewall. | `crates/boyko_shaderdsl/src/scalar.rs`, `cf.rs`, `interp.rs` |
| SDF normal | Central differences at `GRAD_H = 0.0005` (`sdf_field.hlsli:42`; `sdf_normal` at `:232`), mirrored by `boyko_shaderdsl::normal`. It is part of OPT-PLAN's P4 invariant (physics reuses it). Both the Deferred G-buffer composite and Forward's `sdf_forward_march` call it inside the marcher. | `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli` |
| SDF marcher cost model | "one compute thread per pixel". A fully lit pixel can cost about 267 full edit-list folds, which is about 4,300 primitive evaluations at 16 edits. The cost therefore scales with pixel count. | [`../SDF-PERF-AUDIT.md`](../SDF-PERF-AUDIT.md) §1 |

### 0.6 Stale documentation found in the tree

These are recorded here and not fixed; this pass writes only the two post documents.

- **`TaaConfig::jitter_scope` field doc.** It says the default is `RasterOnly`. `impl Default` sets
  `RasterAndBasis`, and the impl's own doc admits the two are "kept in agreement by hand".
- **`SharpenMode` and `SharpenMode::Rcas` docs.** Both say "Declared, NOT wired this rung".
  The sharpen is wired: `present/passes/rcas.rs` exists, and `rcas_sharpness` is documented as a
  push constant the pass reads.
- **The `Rcas` name.** The kernel behind `SharpenMode::Rcas` is AMD CAS (3×3), not FSR 1's RCAS
  (5-tap cross). Its own header says "AMD FidelityFX CAS" (found in the third pass).
- **`AaMode::Taa` doc.** It says "by DEFAULT only the raster mesh path is jittered". That is
  stale against the `RasterAndBasis` default.
- **`TAA-PLAN.md` research basis.** It says "No depth-based history rejection ships". FSR2 ships
  a depth-clip pass [42][F].
- **TAA-PLAN deviations.** TAA-PLAN prescribed YCoCg clipping and a 5-tap history. The tree ships
  RGB (`ClampSpace::Rgb`) and a 16-tap `Load` history. Both deviations are recorded at their
  sites.

---

## 1. Shipped engines at a glance

| Engine | Post surface | Anti-aliasing and upscaling | Where configuration lives |
|---|---|---|---|
| **Unreal Engine 5** [5][7][22][24][40][41] [R] | Histogram auto-exposure plus local exposure; Gaussian bloom plus FFT convolution bloom; Diaphragm DOF; motion blur; a filmic tonemapper with scene-linear grading; lens flare, dirt, CA, vignette, grain | TSR by default. FXAA. TAA. MSAA only in the forward renderer [41]. | Post-process volumes; camera settings |
| **Unity HDRP 14** [23] [F] | A 16-step fixed order (§3). Some effects share one compute shader. | TAA, SMAA and FXAA; DLSS/FSR as upscalers | Volume framework |
| **Bevy 0.19** [6][9][14][31][58][59] [F/R] | `Bloom`, `AutoExposure`, `Tonemapping` (default `TonyMcMapface`), `MotionBlur`, `DepthOfField` | TAA, SMAA, FXAA and CAS; DLSS through `dlss_wgpu` on Vulkan [48] | **Components on the camera entity** |
| **FSR2 integration guide** [42] [F] | Specifies where post goes relative to the upscaler (§3) | FSR 2.2.1, MIT-licensed | — |
| **Frostbite** [11][18] [R/S] | EV100 physical camera; "grade once, output many" HDR display mapping | — | — |
| **Call of Duty: AW** [1] [F] | Scatter-as-gather motion blur and DOF with explicit transparency handling; a pyramidal bloom for temporal stability; separable subsurface scattering | Filmic SMAA T2x [35] | — |

---

## 2. Techniques (T-numbers)

### HDR scene colour

**T1 — Linear HDR scene colour.**
- Every surveyed engine runs exposure, bloom, DOF and motion blur on linear HDR:
  - UE grades in "Scene Referred Linear Space" [22][R].
  - Bevy's bloom requires an HDR camera [6][R].
  - FSR2 wants linear input, with `FFX_FSR2_ENABLE_HIGH_DYNAMIC_RANGE` for HDR [42][F].
- The tree's design for the move is **R4a** in
  [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md), which is also **R11** in
  [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) [T]:
  - `lit` becomes B10G11R11 when the new probe `DeviceCaps::lit_hdr_format_ok` (RK-14) passes,
    otherwise RGBA16F.
  - One `tonemap` pass replaces the eight producer tails.
  - Eight transition pins guard the move, one per producer.
  - R4a says "all 30 pins re-bless". That count is stale. `goldens/PINS.toml` holds **34 pins
    with 61 blessed legs** (34 software + 27 `hwrt`; 7 more `hwrt` legs are `PENDING`), the
    figure its `:39` comment records. Counted here, and found independently by a same-day
    reflections update.
  - It is **owner ballot 1** in both documents, still open.

**T2 — Pre-exposure.**
- This means multiplying radiance by an exposure estimate *before* storing it, so an 11/10-bit
  float target keeps its precision where the image will end up. UE uses it; that fact is recalled,
  not fetched this pass, so it carries no weight here.
- The tree's argument does not depend on UE. B10G11R11 has 5–6 mantissa bits and a finite
  maximum, so scene values need to sit near display range. `LightingConfig::exposure` is already
  the last multiply in every producer [T].
- **Pre-exposure across frames: FSR2's source** [60][F]. FSR2 keeps both `fPreExposure` and
  `fPreviousFramePreExposure` in its constant buffer.
  - `PrepareRgb(rgb, exposure, preExposure)` divides by the pre-exposure, multiplies by the
    exposure, and clamps to `[0, FSR2_FP16_MAX]`.
  - `ReprojectHistoryColor` prepares the history with `PreviousFramePreExposure()`, and
    `UnprepareRgb` re-applies the current `PreExposure()` on output.
  - So a temporal consumer must undo each frame's own pre-exposure. Mixing two frames'
    pre-exposed values is a unit error, and FSR2's code is built to avoid it. The FSR2 README
    [42] defines pre-exposure as the value by which the input is divided to recover the original
    signal (quoted by the design critique; not re-fetched in the third pass).

### Exposure

**T3 — Physical camera exposure, EV100.**
- Bevy's `Exposure` is a camera `Component` [58][F]:
  - Its `exposure()` is `1/(1.2·2^EV100)`, following Filament's physically based camera.
  - Presets: `SUNLIGHT` 15.0, `OVERCAST` 12.0, `INDOOR` 7.0, and `BLENDER` 9.7, "calibrated to
    match Blender's implicit/default exposure".
  - `from_physical_camera` converts aperture, shutter and ISO.
- The EV100 formula in the Frostbite notes [11] could not be fetched (over 10 MB). The standard
  `EV100 = log2(N²/t) − log2(S/100)` is used in the design as a **definition to be pinned by a host
  test**, not as a cited quote.

**T4 — Histogram auto-exposure.**
- **Tardif** [8][F][M]:
  - 256 bins of 4-byte `uint`, built by 16×16 thread groups with groupshared atomics.
  - Log-luminance range −10 to +2, a span of 12. That range suits his content. It is not a
    constant to copy: it fits the tree's display-referred scale by coincidence and would clip a
    physical sun (log2 L ≈ 13). The design derives its own range from the EV clamp.
  - Adaptation: `adapted = last + (target − last)·(1 − exp(−dt·τ))`, with τ = 1.1 recommended.
  - Cost: "a full 1080p HDR target in ~0.17ms on my 2080", against about 0.095 ms for a plain
    reduction.
  - His advice: "use a downsampled HDR target to build the histogram (half size or less), since
    the majority of the cost is in the sheer volume of interlocked histogram accumulation".
- **UE** [7][R]: a 64-bin histogram with low/high percentile filtering and asymmetric speed-up
  and speed-down.
- **Bevy 0.14+** [9][S]: a compute histogram with percentage filtering and a metering mask.
- **Microsoft's Direct2D HDR sample** [19][F] builds its histogram on a 0.5× prescaled image,
  which confirms the downsample practice.

**T5 — Local exposure.**
- UE 5.1+ uses a bilateral grid or exposure fusion [7][R].
- Wronski 2022 [10][R] uses exposure fusion with Laplacian pyramids at quarter resolution. His
  cost of "under 1ms" is an **opinion, not a measurement**. He also records the artefacts: halos
  from Gaussian blending and gradient reversals from bilateral filtering.

### Bloom

**T6 — Bloom pyramid, Jimenez CoD:AW 2014** [1][F][4][R].
- Downsample with a 13-tap filter (about 36 effective bilinear samples).
- Upsample with a 3×3 tent.
- Apply the Karis average `1/(1+luma)` on the first downsample only, against fireflies.
- Use 5–6 mips and no threshold: composite as `mix(hdr, bloom, 0.04)`.
- Store in R11G11B10F.
- Jimenez's page describes the aim as "temporal stability and robustness" [1][F].

**T7 — Dual-filter bloom, Bjørge 2015** [3][S]. A Kawase-style down/up filter aimed at mobile
bandwidth.

**T8 — UE bloom** [5][24][R].
- Standard bloom is Gaussian blurs from 1/2 down to 1/32 resolution.
- FFT convolution bloom is "designed for use with in-game or offline cinematics or high-end
  hardware".
- UE's post page says bloom "Performance cost is radius x radius".

**T9 — Bevy bloom** [6][R][65][F]. An energy-conserving or additive composite, with default
intensity 0.15. How the first mip is sized, from `prepare_bloom_textures` [65][F]:
- `mip_height_ratio = max_mip_dimension / viewport.y`, and the first mip is the viewport scaled
  by that ratio;
- `mip_count = max_mip_dimension.ilog2().max(2) − 1`.

So the chain's footprint is a fixed fraction of the screen, independent of resolution, and its
cost stops growing with resolution. The second pass wrote that the cap makes the look depend on
resolution; that was backwards (§6 C21). The default of 512 is the relayed value [6][R].

### Lens effects

**T10 — Lens effects.**
- **Screen-space pseudo lens flare**, Chapman 2017 [25][R]:
  - A threshold on a downsampled image, then ghosts, a halo, per-channel chromatic offsets, a
    lens-dirt texture and a starburst.
  - The author advises using it "very judiciously" and prefers sprites for prominent flares.
- **UE** [24][R] lists CA, dirt mask, lens flare, vignette and grain. It notes that the flare
  threshold should be "as high as possible to avoid the performance cost".
- **HDRP** [23][F] orders data-driven lens flare, lens distortion, CA, bloom apply, vignette and
  grade apply consecutively (steps 8–13). It states that it "combines some effects into the same
  Compute Shader to minimize the number of passes". **The page does not say which effects share
  a shader.** The researcher's list of fused effects was an inference; §6 records the correction.

### Depth of field

**T11 — Scatter-as-gather DOF.**
- **Jimenez 2014** [1][F]: "scatter-as-you-gather approaches" that "explicitly consider
  transparency".
- **UE Diaphragm DOF**, Abadie 2018 [26][F]. The abstract is one sentence ("output the highest
  bokeh quality while remaining fast"). The page gives **no timing**. The researcher relays:
  physically plausible geometric occlusion, foreground hole filling, scattered sprites for
  highlights, and sub-pixel slight out-of-focus handled together with TAA. UE runs DOF at render
  resolution before TSR [40][R].
- UE's DOF documentation page renders client-side and returned no text to this pass.

**T12 — Separable circular DOF.**
- Garcia 2017 [27][S] uses complex phasors in two passes and shipped in EA sports titles.
- Wronski [28][R]: one component costs about 4N instructions per tap and 6 float channels, and
  rings strongly. Two components cost about 8N and roughly four RGBA16F targets.

**No reliable published ms figure for any DOF technique was found** by either pass.

### Motion blur

**T13 — Motion-blur reconstruction.**
- **McGuire et al. 2012** [29][R]: TileMax (the largest velocity per K×K tile), NeighborMax (a
  3×3 dilation of tiles), then a gather of S jittered samples.
- **Guertin, McGuire & Nowrouzezahrai 2014** [30][S]: "under 2ms at … 1280 × 720" on 2014
  hardware. From a search snippet only.
- **Bevy** [31][R]: `samples*2+1` taps and a `shutter_angle` of 0.5 (≈ 180°). Documented limits:
  fast objects over empty backgrounds do not get blurred edges, and transparents do not blur.
- The casual-effects page for McGuire 2012 returned no text to this pass.

### Tonemapping and grading

**T14 — Analytic tonemap curves.**
- **The tree ships** the Hill ACES fit, Khronos PBR Neutral and Reinhard-Jodie [T].
- **Bevy's descriptions** [14][F]:
  - ACES fitted: "very specific aesthetic, intentional and dramatic hue shifting".
  - Khronos PBR Neutral: "Despite its name, it is not considered to be neutral"; highly
    saturated and high-contrast, aimed at e-commerce.
- **Khronos PBR Neutral** [12][R] reproduces base colour exactly in [0.08, 0.8] under unit white
  light and is invertible.
- **GT tonemap** (Uchimura, CEDEC 2017; SIGGRAPH Asia 2018) [17]: a bibliographic reference only,
  not fetched.

**T15 — LUT-based tonemappers.**
- **AgX** (Sobotka; the Blender 4.0 default [15][R]).
  - Bevy: "very neutral", and "Requires `tonemapping_luts` feature" [14][F].
  - Bevy's source loads `AgX-default_contrast.ktx2` and comments that the AgX LUT is "very small
    (32x32x32)" [59][F].
  - **The formulation's primary source** is Sobotka's repository `config.ocio` [66][F]:
    - a log2 `AllocationTransform` with `vars: [-12.47393, 4.026069]`;
    - an inset `MatrixTransform` whose first row is 0.842479062253094, 0.0784335999999992,
      0.0792237451477643;
    - the contrast curve as a **baked 1D LUT** (`FileTransform {src: AgX_Default_Contrast.spi1d}`),
      not a formula.
  - **No licence statement was found** in the repository's README or page [66][F]. Using the
    curve therefore needs a licence check first.
- **Tony McMapface** (Stachowiak) [13][F].
  - Bevy's current default [14][F].
  - Dual-licensed MIT or Apache-2.0.
  - "intentionally _boring_, does not increase contrast or saturation".
  - Its README does not state the LUT's dimensions or encoding, and says the generator will be
    published "at a later time". Bevy loads it as `tony_mc_mapface.ktx2` [59][F].
  - **The repository's own shader states both** [64][F]: `const float LUT_DIMS = 48.0;` and the
    input encoding `stimulus / (stimulus + 1.0)`, aligned to texel centres before a
    `SampleLevel`. The second pass's "not published" was wrong (§6 C20).
- **ACES 2.0** [16][R]: community advice is to bake a 64³ or 65³ LUT in compute. The same thread
  reports "breakage in the blue and the yellow hues".

**T16 — Colour grading.**
- UE grades in scene-linear and flags legacy LDR LUTs as not HDR-compatible [22][R].
- HDRP bakes a grading LUT at step 7 and applies it at step 13 [23][F].
- Fry (GDC 2017) [18][S]: "grade once, output many".

### HDR display output

**T17 — HDR display output.**
- **Windows** [19][F]:
  - **Option 1** is an FP16 scRGB swapchain: "the only option that works for all types of
    Advanced Color displays", but it "doubles GPU bandwidth and memory consumption".
  - **Option 2** is R10G10B10A2 with HDR10/BT.2100 PQ. It requires an HDR display, D3D11/12 and no
    alpha-blended swapchain. It "consumes the same 32 bits per pixel". Note that these conditions
    are DXGI's and are stated for D3D.
  - On an HDR display, scRGB 1.0 is 80 nits. SDR reference white is "typically set to around 200
    nits" on desktop monitors.
  - A Win32 app reads SDR white through `QueryDisplayConfig` / `DISPLAYCONFIG_SDR_WHITE_LEVEL`,
    reads display capability through `IDXGIOutput6::GetDesc1` (`DXGI_OUTPUT_DESC1`), and must
    poll `IDXGIFactory1::IsCurrent` because it has no change event.
  - On an SDR display, an FP16 swapchain is clipped to [0, 1].
  - Named tonemap options: ACES Filmic, Reinhard and the "ITU-R BT.2390-3 EETF".
- **Vulkan**: `VK_EXT_swapchain_colorspace` adds `HDR10_ST2084` and `EXTENDED_SRGB_LINEAR` [20][R].
  `VK_EXT_hdr_metadata` sets SMPTE 2086 / CTA 861.3 metadata, and the presentation engine "may
  process the image based on the metadata" [21][R].
- **Not sourced:** whether Windows Vulkan drivers expose `HDR10_ST2084` only while Windows HDR is
  enabled. The design's pass 1 asserted it; no source was found, and the claim is withdrawn.

### Sharpening

**T18 — CAS and RCAS** [32][F][61][F].
- CAS is MIT-licensed and "designed to help increase the quality of existing Temporal
  Anti-Aliasing (TAA) solutions". It can scale and sharpen in one pass.
- The page gives no timing.
- FSR2 ends with RCAS [42][F].
- **RCAS, from FSR 1's `ffx_fsr1.h`** [61][F]:
  - "RCAS uses a 5 tap filter in a cross pattern (same as CAS)";
  - `hitMin = min(mn4, e) · rcp(4·mx4)` and `hitMax = (peakC.x − max(mx4, e)) · rcp(4·mn4 +
    peakC.y)`;
  - `lobe = max(−hitMin, hitMax)`, limited by `FSR_RCAS_LIMIT (0.25-(1.0/16.0))`;
  - the output is `(lobe·(b + d + h + f) + e) · rcpL`;
  - **"Each channel needs to be in the range [0, 1]"**, so raw HDR input is out of its contract.
- **The tree's `SharpenMode::Rcas` pass is CAS, not RCAS** [T] (§0.2): a 3×3 of 9 taps, with the
  CAS amplitude limiter.

### Spatial anti-aliasing

**T19 — FXAA, CMAA2 and SMAA 1x** [33][F][M]. Intel's CMAA2 article, Table 2, added cost in ms:

| GPU | Resolution | FXAA | CMAA2 | SMAA |
|---|---|---|---|---|
| GTX 1080 | 1080p | 0.11 | 0.15 | 0.30 |
| GTX 1080 | 4K | 0.37 | 0.38 | 0.98 |
| Vega 64 | 1080p | 0.09 | 0.10 | 0.23 |
| Vega 64 | 4K | 0.30 | 0.24 | 0.80 |
| Intel NUC8i7HVK | 1080p | 0.24 | 0.22 | 0.64 |
| Intel NUC8i7HVK | 4K | 0.82 | 0.59 | 2.12 |

- PSNR against a supersampled reference: CMAA2 36.86, FXAA 36.75, SMAA 36.67.
- CMAA2 is three compute passes: edge detection, shape processing and resolve.
- The standalone CMAA2 "does not yet include" temporal stability; TSCMAA is a separate variant.
- The Intel page states no licence. The researcher relays Apache-2.0 from the GitHub repository,
  which was archived in 2023 [33][R].
- SMAA is MIT-licensed [34][R].
- Filmic SMAA T2x was "engineered to meet … 0.9-1.05ms @1080p" on an unnamed platform [35][R].

**T20 — MSAA with deferred or visibility-buffer shading.**
- UE: MSAA is "only available … when using the Forward Renderer" [41][R].
- Engel [55][M][R], memory at 1080p:

| Samples | Visibility buffer | G-buffer |
|---|---|---|
| 1× | 8 MB | 20 MB |
| 2× | 16 MB | 40 MB |
| 4× | 32 MB | 80 MB |

- Drobot's HRAA uses EQAA coverage on console hardware [2][R].
- In the tree, the SDF marcher and the deferred resolve are single-sample compute, so MSAA
  **cannot reach an SDF pixel** [T].

### Temporal anti-aliasing and upscaling

**T21 — TAA.**
- **Karis 2014** [2][R]: temporal supersampling "does not require MSAA". His 2013 post [37][R]
  gives the tonemap-weighted resolve `w = 1/(1+luma)`.
- **Playdead's INSIDE** [38][R], MIT-licensed: clip toward the centre, velocity-weighted feedback
  and closest-depth dilation.
- **Jimenez 2017** [36][R]: dynamic AA degradation under load.
- **Decima 2017** [36][R]: a 2-frame TAA and a checkerboard resolve.
- **Survey:** Yang, Liu & Salvi, CGF 2020 [39], not fetched.

**T22 — FSR 2.2.1** [42][F][V].
- **Licence:** MIT.
- **Stages:** "Compute luminance pyramid, Reconstruct & dilate, Depth clip, Create locks,
  Reproject & accumulate, Robust Contrast Adaptive Sharpening (RCAS)".
- **Jitter phase counts:** Quality 1.5× → 18; Balanced 1.7× → 23; Performance 2× → 32; Ultra
  Performance 3× → 72.
- **Mip bias:** `mipBias = log2(renderResolution/displayResolution) - 1.0`.
- **Vulkan FP16** needs `shaderFloat16` and `storageBuffer16BitAccess`, with an FP32 fallback.
- **Masks:**
  - Reactive: "0.0 indicates pixel is not reactive".
  - Transparency & composition: "value of 1 removes the lock completely".
- **Performance table** (FSR 2.2.1) [V], in ms:

| GPU | 1440p Quality | 1440p Performance | 4K Quality | 4K Performance |
|---|---|---|---|---|
| RX 7900 XTX | 0.3 | 0.3 | 0.7 | 0.6 |
| RX 6800 XT | 0.5 | 0.4 | 1.2 | 1.0 |
| RX 6700 XT | 0.9 | 0.8 | 2.0 | 1.7 |
| **RX 6650 XT** | **1.2** | **0.9** | **2.8** | **2.3** |
| RX 5700 XT | 1.1 | 0.9 | 2.4 | 2.0 |
| RX 590 | 2.3 | 1.9 | 5.4 | 4.4 |

- **Memory** (working / persistent / aliasable, MB) [V]:

| Resolution | Quality | Performance |
|---|---|---|
| 1440p | 207 / 164 / 43 | 172 / 143 / 29 |
| 4K | 448 / 354 / 93 | 376 / 312 / 63 |

**T23 — Other upscalers.**
- **FSR SDK 2.3.0** [43][R] (FSR 3.1.5, FSR 4.1.1): its README lists "Vulkan is currently not
  supported in SDK".
- **FSR 4** [44][R]: "only as prebuilt, signed DLLs"; DX12 only; RX 7000/9000. 1.3 ms at 4K
  Performance on an RX 9070 XT [V].
- **DLSS** [45][R]: NVIDIA RTX SDKs licence, object code only, NVIDIA marks on splash and about
  screens, NVIDIA must be notified before release, "only for … systems with NVIDIA GPUs".
  Streamline lists DX11 and Vulkan 1.2+ [46][R]. Bevy 0.17 integrates DLSS through `dlss_wgpu`
  on Vulkan [48][R].
- **XeSS** [47][R]: Intel Simplified Software License; binary only; no modification.
- **UE TSR** [40][R]:
  - History at 200% of display resolution at Epic/Cinematic, 100% below. `r.TSR.History.R11G11B10`.
  - 0.79 ms at 100% and 0.43 ms at 50% screen percentage (Valley of the Ancient; the platform was
    not captured).
  - About 1.5 ms on PS5/XSX in Fortnite Chapter 4, with about 0.5 ms hidden by async compute.

### Aliasing sources beyond the edge

**T24 — Specular AA.**
- Kaplanyan, Hill, Patney & Lefohn, HPG 2016 [50][R]: "compatible with deferred shading, normal
  maps".
- Tokuyoshi & Kaplanyan, I3D 2019 [51][R].
- JCGT 2021 [52]: the page returned no text to either pass, so **its title and content are not
  verified**.
- Filament's per-material defaults: `specularAntiAliasingVariance` 0.15 and
  `specularAntiAliasingThreshold` 0.05 [53][R].
- The methods need screen-space normal derivatives. The SDF marcher is a 1-D dispatch with "no
  hardware ddx/ddy" [T] (`RENDER-C-AA-DESIGN-PARKED.md`).
- Toksvig and LEAN were not fetched by either pass.

**T25 — SDF-edge aliasing.**
- Dreams runs "almost entirely on the PS4's compute unit" [57][R] and uses TAA as the
  reconstruction filter.
- In the tree, `RasterAndBasis` jitters the SDF rays. `JitterScope::RasterOnly`'s doc records TAA
  as a structural no-op on SDF pixels in that mode (0 of 810,000 pixels differed) [T].
- Analytic SDF coverage is designed and **parked** (`RENDER-C-AA-DESIGN-PARKED.md`) [T].

**T26 — Alpha-test aliasing.**
- Hashed alpha testing, Wyman & McGuire I3D 2017 [54][R].
- Filament enables alpha-to-coverage for `masked` materials [53][R], which requires MSAA.
- The tree has no alpha-test path. The transparency design's R2 `MASKED` variant uses a hashed
  threshold [T].

### Surrounding practice

**T27 — Where post settings live.**
- Bevy puts `Bloom`, `MotionBlur`, auto-exposure, `Tonemapping` and `Exposure` on the **camera
  entity** as components [6][31][9][14][58].
- UE and HDRP blend volumes; their volume-blending rules were not verified this pass.

**T28 — Single-pass mip reduction, SPD** [56][F]: "up to 12 mip levels (maximum source texture
size is 4096x4096)"; wave operations optional. The page gives no timing.

**T29 — Dynamic resolution.** CAS supports DRS scaling in its sharpening pass [32][F]. HDRP lists
dynamic resolution [23][F]. The usual implementation keeps render targets at a maximum size and
renders into a sub-rectangle, so a scale change needs no reallocation. Any sampled read near the
rectangle's edge must then clamp to it. That description is recalled, not sourced. The tree
precedent for a render extent that differs from the display extent is SSAA: `aa_extent` is native
while `present_extent` is 2× (`GBufferTargets::aa_out`'s doc) [T].

**T30 — Film grain and dithering.** HDRP runs them last, as steps 15 and 16 ("8-bit Dithering"),
after FXAA [23][F]. HDRP can dither last because its colour stays in a float format until that
final pass. Dithering is only useful at the point where a higher-precision value is quantised; the
tree's `display` is RGBA8, so its quantisation point is the pass that writes `display`.

**T31 — Spatial upscaling: FSR 1 (EASU + RCAS)** [62][F][V][61][F].
- **Licence:** "Provided on GPUOpen under an MIT license".
- **What it is:** "a spatial upscaler: it works by taking the current anti-aliased frame and
  upscaling it to display resolution without relying on other data such as frame history or
  motion vectors". It is two passes, EASU (edge-adaptive spatial upsampling) and RCAS.
- **Placement:** FSR 1 passes "work best in perceptual color space, and should therefore be
  integrated after tone mapping". Passes "that introduce noise or other high-frequency visual
  components to the scene should be rendered after upscaling to avoid those noisy components
  being amplified".
- **Scale factors:** Ultra Quality 1.3× per axis (77 %), Quality 1.5× (67 %), Balanced 1.7×
  (59 %), Performance 2.0× (50 %).
- **Cost ceilings [V]:**

| GPU class | 1440p Ultra Quality → Performance | 4K Ultra Quality → Performance |
|---|---|---|
| RX 6800 XT, RTX 3080 | ≤ 0.20 → ≤ 0.50 ms | ≤ 0.40 → ≤ 1.0 ms |
| RX 6700 XT, RTX 3060 Ti | ≤ 0.30 → ≤ 0.50 ms | ≤ 0.60 → ≤ 1.0 ms |

- Neither survey pass mentioned FSR 1 before the critique (§6 C18).

---

## 3. Chain order in shipped engines

| Stage | HDRP 14 [23][F] | UE5 [40][22][R] | FSR2 guide [42][F] | boyko today [T] |
|---|---|---|---|---|
| NaN killer | Step 1 | — | `PrepareRgb` clamps inputs to `[0, FP16_MAX]` [60] | **None.** There is no clamp in TAA; the only protection is implicit, from the RGBA8 UNORM store (§0.2). The second pass's "Clamp inside TAA" was wrong (§6 C14). |
| Temporal AA / upscale | **Step 2** | After DOF | After "post processing A" (SSR, SSAO, denoisers) | After `particle_draw`, **on 8-bit post-tonemap** |
| DOF | Step 3, after TAA | **Before** TSR, render resolution | **After** (presentation resolution) | — |
| Motion blur | Step 4 | After TSR | **After** | — |
| Bloom | Step 6 pyramid, step 11 apply | After TSR | **After** (presentation resolution) | — |
| Grading and tonemap | Step 7 bake, step 13 apply | Tonemapper at display resolution | After | Inside the eight producers |
| Lens flare / distortion / CA / vignette | Steps 8–12 | After TSR | CA and vignette after | — |
| FXAA | Step 14, **after** grading | Post | — | Reads 8-bit `lit` |
| Grain / dither | Steps 15 and 16 (last) | Late | Grain after | — |
| Spatial upscale (FSR 1) | — | — | — | — (FSR 1's own guidance [62]: after tonemapping, before noise) |

The three sources **agree** on five points:
- the temporal resolve comes first;
- bloom, CA, vignette and grain come after it;
- tonemapping and grading come after it;
- spatial AA runs on the LDR, graded image;
- grain and dither come last.

The **one split is DOF**. UE runs it before TSR, at render resolution, because DOF is expensive
per pixel and TSR can absorb its sub-pixel noise. HDRP and FSR2 run it after the temporal
resolve, at display resolution.

---

## 4. The numbers, and the bandwidth model built from them

### 4.1 Published figures

| Figure | Value | Rig | Kind |
|---|---|---|---|
| Histogram, full 1080p | 0.17 ms (plain reduction 0.095 ms) | RTX 2080 | [M] [8] |
| FXAA / CMAA2 / SMAA | §2 T19 table | GTX 1080, Vega 64, Intel NUC | [M] [33] |
| FSR 2.2.1 | §2 T22 table | AMD RDNA 1–3 | [V] [42] |
| FSR 4, 4K Performance | 1.3 ms | RX 9070 XT | [V] [44] |
| UE TSR | 0.79 / 0.43 ms at 100% / 50% screen percentage | Not captured | [V] [40] |
| UE TSR in Fortnite | ≈ 1.5 ms, ≈ 0.5 ms hidden by async compute | PS5 / XSX | [V] [40] |
| Motion blur (Guertin 2014) | < 2 ms at 1280×720 | 2014 GPU | [S] [30] |
| Filmic SMAA T2x | 0.9–1.05 ms at 1080p | Unnamed | [R] [35] |
| MSAA memory | §2 T20 table | 1080p | [M] [55] |
| In-tree TAA | 0.30–0.45 ms at 1080p | RTX 3060, derived | [E] `TAA-PLAN.md` |
| FSR 1 (EASU + RCAS) | §2 T31 table | AMD / NVIDIA classes | [V] [62] |
| RTX 3060 Laptop | 120 TMUs; boost 1283–1703 MHz; "166.4 (204.4)" GT/s; "336 (288)" GB/s; "6.912 (10.94)" TFLOPS; 60–115 W | spec table | [F] [63] |
| RTX 3060 (desktop) | 112 TMUs; boost 1777 MHz; "147.8 (199)" GT/s; 360 GB/s | spec table | [F] [63] |
| RTX 3060 Ti | boost 1665 MHz; 253.1 GT/s; 448 GB/s | spec table | [F] [63] |

### 4.2 Bandwidth model

**Pixel counts.** 1080p is 2.074 Mpx, 1440p is 3.686 Mpx and 4K is 8.294 Mpx.

**The reference GPU is the RTX 3060 Laptop (6 GB)**, per `OPTIMIZATION-PLAN-RENDER.md:44`
("GPU oracle = RTX 3060 Laptop (6 GB)"). The second pass costed against the desktop RTX 3060, and
that was wrong (§6 C17). The two parts differ:
- the laptop part has 30 SMs and 120 TMUs, against the desktop's 28 and 112;
- the laptop part's clocks are lower and depend on its 60–115 W power setting;
- the laptop part's bandwidth is 336 GB/s, or 288 GB/s on a variant [63][F].

**Assumed bandwidth.** The in-tree figure is **250 GB/s effective**. It is `TAA-PLAN.md`'s m6
assumption and is itself not measured. It is 74 % of the laptop part's 336 GB/s and 87 % of its
288 GB/s variant, which makes it optimistic there: on that variant every DRAM floor below is ×1.17.
At 250 GB/s, each byte per pixel costs:

| Resolution | ms per B/px |
|---|---|
| 1080p | 0.0083 |
| 1440p | 0.0147 |
| 4K | 0.0332 |

A pass's bandwidth floor [E] is its bytes per pixel times that figure.

**Assumed texel rate.** The laptop part's 120 TMUs give "166.4 (204.4)" GT/s [63][F]: 166.4 at the
top of its base-clock range (1387 MHz), and 204.4 at its top boost clock. Across its power
configurations the boost clock spans 1283–1703 MHz, i.e. 154–204 GT/s. The model assumes a
sustained **166 GT/s** for 32-bit bilinear fetches. Each fetch per pixel then costs:

| Resolution | ms per fetch per pixel |
|---|---|
| 1080p | 0.0125 |
| 1440p | 0.0222 |
| 4K | 0.0500 |

**Per-format assumptions**, PLAUSIBLE and unmeasured (P29):
- a filtered 64-bit fetch (RGBA16F bilinear) is half rate, so it counts as 2;
- an unfiltered `Load` counts as 1 at either width;
- a 3D trilinear fetch counts as 2, and 4 if it is 64-bit.

The design's PX0 carries a bench-only probe that measures all of these on the owner's GPU.

**A pass's estimate** is `[max(DRAM floor, texel floor), DRAM floor + texel floor]`, plus the
dispatch overhead. The lower end assumes the two streams overlap fully and the upper end assumes
they do not overlap at all. The second pass costed most passes on DRAM alone, which left out the
fetch term of every filter and gather (§6 C17). The design's §6.1 lists both terms per pass.

**Scaling vendor numbers to the laptop part.** Ratios are taken from the fetched table:
- the RTX 3060 Ti's texel rate is 253 / 166 = 1.52× the laptop part's sustained rate;
- the GTX 1080 is used only as a range. Its fetched extract was internally inconsistent (a
  320 GT/s fill rate beside a 1733 MHz base clock), so its measured FXAA, SMAA and CMAA2 costs
  are carried at ×1.0–1.9 [E].

**FSR 2.2.1's RX 6650 XT row is not a proxy.** The second pass called it the "nearest published
proxy" for the reference GPU. It is another vendor's GPU, running an upscaler that shades 0.44× the
pixels, so it anchors nothing in the design's estimates. It remains only as a vendor figure (§6 C23).

---

## 5. Pitfalls (P-numbers)

1. **Bloom fireflies and pulsing.** Apply the Karis average on the first downsample [4][1].
   Thresholds are non-physical [4][6].
2. **Exposure over clipped input.** Exposure and bloom computed on today's 8-bit post-tonemap
   `lit` see clipped values [T].
3. **Metering stability.** Extremes must be percentile-filtered [7][8]. Adaptation should be
   asymmetric [7]. Atomic contention calls for a downsampled source [8].
4. **Local tonemapping** produces halos or gradient reversals [10].
5. **Tonemapper hue behaviour.**
   - ACES fit shifts hues [14].
   - Filmic has "the notorious six" [15][S].
   - ACES 2.0 has reported blue and yellow breakage [16].
   - Khronos Neutral is exact only in [0.08, 0.8] [12].
6. **HDR output.**
   - UI looks dim without SDR-white scaling [19].
   - DWM clips out-of-range values when the display is in SDR mode [19].
   - A Win32 app has no change event and must poll `IsCurrent` [19].
   - The presentation engine may alter the image according to metadata [21].
7. **Translucents and TAA.** Translucents write no velocity [40]. Particles and alpha need a
   reactive mask [42]. In the tree, `particle_draw` composites into `lit` before `taa_resolve`
   with no motion vectors and no mask [T].
8. **Mip bias under TAA.** A negative mip bias is needed under jitter [42]. The tree has none [T].
9. **Thin geometry** flickers under TAA. UE added flicker rejection [40].
10. **Disocclusion.** The tree's TAA tests off-screen samples only [T]. FSR2 has a depth-clip
    pass [42].
11. **Resolve space.** Blend in tonemapped or luma-weighted space [37]. On HDR input the weighting
    is scale-dependent, so exposure must reach the resolve (T2). The history must also be in the
    same pre-exposure units as the current frame (P26).
12. **DOF and TAA fight** over sub-pixel slight out-of-focus [26].
13. **Motion blur edges.** Fast objects over static or empty backgrounds stay sharp-edged [31].
    Tile dilation addresses this [29].
14. **Spatial AA limits.** FXAA blurs. Base CMAA2 is not temporally stable [33].
15. **Moving SDF bodies** get no motion vectors off `hwrt` [T].
16. **Licensing traps.** DLSS requires notification and attribution [45]. FSR 4 ships as signed
    DLLs [44]. The FSR SDK 2.3.0 does not support Vulkan [43].
17. **Unmeasurable cost.** There are no post GPU zones, so no post cost claim can be checked
    today [T].
18. **Golden churn.** The HDR move re-blesses every blessed leg: 61 at `6394bc5e`, not R4a's
    "30" (T1). A single deferred-path pin would miss
    a producer that double-tonemaps [T] (R4a).
19. **Invisible AA output.** `aa_out` lives outside the framegraph. A new pass that reads or
    writes it would get no derived barrier and would need hand-recording [T].
20. **Missing path term.** `targets.rs` arms `aa_out` with no path term. A Forward path with an
    armed `aa_out` would present an uninitialised image; `post_process_aa_supported`'s doc says
    so [T].
21. **Formatless storage writes are unavailable.** `shaderStorageImageWriteWithoutFormat` is not
    enabled, so an HDR storage writer needs one `.spv` per storage format (B10G11R11 or RGBA16F
    under RK-14) [T].
22. **Transcendentals and the host oracle.** The eDSL has no `log2`/`exp2`/`pow`. Adding them
    gives GPU results that are ULP-approximate against an `f32` host oracle, so post leaves are
    compared at 8-bit output resolution, not bit-for-bit [T].
23. **Dither and grain versus goldens.** Both break byte-identity by design. Their noise must be
    keyed on the frame serial so that goldens stay deterministic.
24. **Mip bias costs bandwidth.** A −1 bias quadruples the texel footprint of minified fetches.
    The VB shading path samples with `SampleGrad` [T], so the bias is applied by scaling the
    gradients, and its cost appears in the VB shade zone.
25. **NaN and Inf persist in temporal state.** An 8-bit UNORM store removes NaN implicitly; HDR
    formats keep it [T] (§0.2).
    - In the TAA history it spreads through the Catmull-Rom sum every frame, and the clip does not
      remove it, so the region grows without decaying.
    - In a cross-frame exposure state it blacks the image permanently.
    - HDRP's step 1 is a NaN killer [23], and FSR2 clamps every input [60].
26. **Pre-exposure across frames.** Pre-exposure is exact within a frame and a unit error across
    frames. A history in the previous frame's units, blended with the current frame, is wrong by
    the ratio of the two pre-exposures. FSR2 undoes each frame's own pre-exposure [60] (T2).
27. **A physical camera on a non-physical scale.** EV100 presets assume physically scaled light.
    On the tree's display-referred scale, `f/16, 1/100 s, ISO 100` renders about 2^14.9 too dark,
    and a histogram range copied from another engine fits only by coincidence [T] (§0.1, T4).
28. **Noise before a spatial pass.** Grain fed into FXAA or SMAA is detected as edges; FSR 1
    requires noise after upscaling [62]. Dither is the exception: it must sit where
    higher-precision data is quantised (T30).
29. **Texel rate depends on the format.** A filtered 64-bit fetch is commonly half rate. That is
    PLAUSIBLE and was not measured or sourced here, and it can double the texel term of a gather
    that reads RGBA16F (§4.2).

---

## 6. Corrections to, and extensions of, the relayed research

| # | Relayed claim | What this pass found | Evidence |
|---|---|---|---|
| C1 | "The FSR2 guide: bloom before the upscaler" (TL;DR and the §2.18 table) | **Wrong.** The README puts bloom in "Post processing B", which it recommends to "run after FSR2, … at the larger, presentation resolution", together with film grain, CA, vignette, tonemapping, DOF and motion blur. | [42][F], fetched twice |
| C2 | HDRP fuses "lens flare, lens distortion, CA, bloom apply, vignette and grade" in one compute pass | The page says only that "some effects" share a compute shader. The list was inferred from the order. | [23][F] |
| C3 | FSR2 per-GPU figures | Confirmed row by row. Version 2.2.1. | [42][F] |
| C4 | Tardif 0.17 ms | Confirmed. Added: τ = 1.1, log range −10…+2, the downsample advice quoted. | [8][F] |
| C5 | Intel table | Confirmed. Added: the Intel NUC rows. The Intel page states no licence. | [33][F] |
| C6 | "Add a previous-depth binding" for TAA disocclusion | **No new image is needed.** `viewt` is FIF-ringed, so `viewt[1-fi]` is the previous frame's depth proxy; only a binding and a cross-frame seed are missing. | §0.3 [T] |
| C7 | (not in the report) | `aa_out` is outside the framegraph; the graph is re-declared every frame; one camera renders per frame; the eDSL has no `log2`/`exp2`/`pow`; storage writers pin formats and formatless writes are off; no 3D storage image exists yet. | §0 [T] |
| C8 | Bevy exposure | Added the `Exposure` component, `1/(1.2·2^EV100)` and its presets. | [58][F] |
| C9 | Tony McMapface and AgX LUTs | The Tony LUT's dimensions are **not published** in its README. Bevy's source states the AgX LUT is 32³. | [13][59][F] |
| C10 | Windows HDR | Added: Win32 reads SDR white through `QueryDisplayConfig`; the Option-2 conditions are DXGI/D3D conditions; the Direct2D sample builds its histogram at 0.5× scale. | [19][F] |
| C11 | "E-EXP and E-MBLUR need P6-S — open" | No code carries a P6-S label. TAA and DDGI already carry cross-frame state on the single queue through seeded resources. The design closes the question (its §1). | [T] |
| C12 | (from R4a, not the report) "all 30 pins re-bless" | `goldens/PINS.toml` holds 34 pins and **61 blessed legs** (34 software + 27 `hwrt`, 7 `hwrt` `PENDING`). | [T], counted |

**Third pass: corrections found by the design critique, or while answering it.** Each row names
the critique remark it answers.

| # | What the second pass or the design said | What is true | Evidence |
|---|---|---|---|
| C13 | Pre-exposure "affects only precision, never the result" (C1) | True within a frame only. Across frames, a temporal consumer must undo each frame's own pre-exposure. | [60][F]: `PrepareRgb`, `ReprojectHistoryColor`, `fPreviousFramePreExposure` |
| C14 | §3: "NaN killer: Clamp inside TAA"; the design's `clamp(out, 0, 64)` (W1) | No clamp exists. The protection is the implicit RGBA8 UNORM store, which HDR removes. | [T]: `taa_resolve.comp.hlsl:410`, `:519`, grep |
| C15 | SSAA keeps RGBA8 `aa_out` as its output (W2) | R4a's `lit(2×) → lit(native)` needs a named native HDR image. | [T]: R4a; `GBufferTargets::aa_out`'s doc |
| C16 | The tree's sharpen is "RCAS", a 5-tap cross, and the fused path can match it within 1/255 (W3) | The tree's kernel is CAS (3×3). Fused and standalone differ by up to about 6/255 at high-contrast edges [E], because both lobes depend on min/max ratios that a nonlinear remap changes. | [T]: `rcas.comp.hlsl:99`, `:105`; [61][F] |
| C17 | Costs on "the RTX 3060's nominal bilinear rate" of ~200 GT/s; most passes costed on DRAM alone (W6) | The reference GPU is the RTX 3060 Laptop, at 166 GT/s sustained. Every filter and gather carries a texel term. | [T]: `OPTIMIZATION-PLAN-RENDER.md:44`; [63][F] |
| C18 | (absent) spatial upscaling; dynamic resolution surveyed and then closed silently (W7) | FSR 1 is MIT, spatial and cheap. The design now races it, and decides dynamic resolution. | [62][F][V] (T31) |
| C19 | Grain and dither before spatial AA "as HDRP does" (W8) | HDRP puts them after FXAA. Grain must follow spatial passes; dither must sit at the quantisation, which in the tree is the `display` write. | [23][F]; [62][F]; [T]: `fxaa.fs.hlsl:48-49`, `smaa_common.hlsli:41` |
| C20 | The Tony LUT's size and encoding are not published (O2) | 48³, with input encoded as `x/(x+1)`, stated in the repository's own shader. | [64][F] |
| C21 | Bevy's 512-px cap makes the look resolution-dependent (O3) | Backwards: the first mip is a fixed fraction of the viewport. | [65][F] |
| C22 | Tardif's −10…+2 used as the histogram range; the Bevy EV presets as test targets (W5) | The tree's light scale is display-referred, so the range must be derived and physical presets wait for physical units. | [T]: `light.rs:316`, `forward_mesh.rs:125`, `csm_fit_eval.rs:74` |
| C23 | The RX 6650 XT row is the "nearest published proxy" for the reference GPU (Q7) | It is another vendor's GPU running an upscaler that shades 0.44× the pixels. It is not a proxy. | reasoning; [42] |
| C24 | AgX "generated from its published formulation" (Q5) | Its matrix and log2 range are published in `config.ocio`, but its curve is a baked 1D LUT, and no licence statement was found. | [66][F] |
| C25 | Vulkan drivers expose `HDR10_ST2084` "when OS HDR is on" (Q6) | Not sourced; withdrawn. | — |
| C26 | New transcendentals on a separate `PostScalar: FieldScalar` trait (W4) | The tree places transcendentals on `Cf`, with an `EvalCf` `nightly`/`std` shim, to keep them off the physics leaf. | [T]: `cf.rs` `Cf::sin`, `interp.rs` |
| C27 | "`vkCmdDispatchIndirect`, which exists" (O1) | The raw function exists, but the RHI trait method is a silent no-op that the Vulkan backend does not override. | [T]: `encoder.rs:408` |

---

## 7. What neither pass verified

These carry no weight in the design unless re-fetched:

- The Frostbite EV100 derivation [11].
- The UE DOF and motion-blur documentation pages, which render client-side.
- The DLSS guide.
- The JCGT 2021 paper [52].
- Toksvig and LEAN.
- The GT tonemap formula [17].
- ACES 2.0's output transform [16] beyond the ACEScentral advice.
- Any measured DOF cost.
- Any measured motion-blur cost on current hardware.
- The AgX repository's licence (none found, [66]).
- Whether Vulkan drivers tie `HDR10_ST2084` to Windows HDR being enabled.
- The filtered 64-bit texel rate on the reference GPU (P29).
- The Vulkan specification's text for float → UNORM conversion of NaN, and its mandatory
  format-support table (the fetched page returned neither).
- The GTX 1080's texel rate (its fetched extract was inconsistent).

The Tony McMapface LUT size, listed here after the second pass, is now verified (T15, [64]).

---

## Sources

[1] https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/ — Jimenez 2014 [F]
[2] https://advances.realtimerendering.com/s2014/index.html — Karis TAA, Jimenez, Drobot HRAA [R]
[3] https://community.arm.com/cfs-file/__key/communityserver-blogs-components-weblogfiles/00-00-00-20-66/siggraph2015_2D00_mmg_2D00_marius_2D00_slides.pdf — Bjørge dual filter [S]
[4] https://learnopengl.com/Guest-Articles/2022/Phys.-Based-Bloom — 13-tap / tent / Karis-average bloom [R]
[5] https://dev.epicgames.com/documentation/en-us/unreal-engine/bloom-in-unreal-engine — UE bloom [R]
[6] https://docs.rs/bevy/latest/bevy/post_process/bloom/struct.Bloom.html — Bevy bloom [R]
[7] https://dev.epicgames.com/documentation/unreal-engine/auto-exposure-in-unreal-engine?lang=en-US — UE exposure [R]
[8] https://www.alextardif.com/HistogramLuminance.html — histogram exposure [F]
[9] https://bevy.org/news/bevy-0-14/ — Bevy auto exposure [S]
[10] https://bartwronski.com/2022/02/28/exposure-fusion-local-tonemapping-for-real-time-rendering/ — exposure fusion [R]
[11] https://seblagarde.wordpress.com/2015/07/14/siggraph-2014-moving-frostbite-to-physically-based-rendering/ — Frostbite PBR (the notes PDF exceeds the fetch limit)
[12] https://github.com/KhronosGroup/ToneMapping/blob/main/PBR_Neutral/README.md — Khronos PBR Neutral [R]
[13] https://github.com/h3r2tic/tony-mc-mapface — Tony McMapface [F]
[14] https://docs.rs/bevy/latest/bevy/core_pipeline/tonemapping/enum.Tonemapping.html — Bevy 0.19.1 tonemappers [F]
[15] https://developer.blender.org/docs/release_notes/4.0/color_management/ — AgX in Blender 4.0 [R]
[16] https://github.com/aces-aswf/aces-output ; https://community.acescentral.com/t/how-to-integrate-aces-2-0-into-a-game-engine/5807 — ACES 2.0 [R]
[17] https://dl.acm.org/doi/10.1145/3277644.3277791 — GT Sport HDR (not fetched)
[18] https://gdcvault.com/play/1024466/High-Dynamic-Range-Color-Grading — Fry, Frostbite HDR [S]
[19] https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range — Windows Advanced Color [F]
[20] https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_EXT_swapchain_colorspace.html [R]
[21] https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_hdr_metadata.html [R]
[22] https://dev.epicgames.com/documentation/en-us/unreal-engine/color-grading-and-the-filmic-tonemapper-in-unreal-engine [R]
[23] https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@14.0/manual/Post-Processing-Execution-Order.html [F]
[24] https://dev.epicgames.com/documentation/en-us/unreal-engine/post-process-effects-in-unreal-engine [R]
[25] https://john-chapman.github.io/2017/11/05/pseudo-lens-flare.html [R]
[26] https://advances.realtimerendering.com/s2018/index.htm — Abadie, "A Life of a Bokeh" [F, abstract only]
[27] https://dl.acm.org/doi/10.1145/3084363.3085022 — Garcia 2017 [S]
[28] https://bartwronski.com/2017/08/06/separable-bokeh/ [R]
[29] https://dl.acm.org/doi/10.1145/2159616.2159639 — McGuire 2012 [R]
[30] https://doi.org/10.2312/hpg.20141093 — Guertin 2014 [S]
[31] https://docs.rs/bevy/latest/bevy/post_process/motion_blur/struct.MotionBlur.html [R]
[32] https://gpuopen.com/fidelityfx-cas/ [F]
[33] https://www.intel.com/content/www/us/en/developer/articles/technical/conservative-morphological-anti-aliasing-20.html [F] ; https://github.com/GameTechDev/CMAA2 [R]
[34] https://github.com/iryoku/smaa ; https://www.iryoku.com/smaa/ [R]
[35] https://advances.realtimerendering.com/s2016/ ; https://research.activision.com/publications/archives/filmic-smaasharp-morphological-and-temporal-antialiasing [R]
[36] https://advances.realtimerendering.com/s2017/index.html [R]
[37] http://graphicrants.blogspot.com/2013/12/tone-mapping.html [R]
[38] https://github.com/playdeadgames/temporal [R]
[39] https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.14018 (not fetched)
[40] https://dev.epicgames.com/documentation/en-us/unreal-engine/temporal-super-resolution-in-unreal-engine [R]
[41] https://dev.epicgames.com/documentation/unreal-engine/anti-aliasing-and-upscaling-in-unreal-engine [R]
[42] https://github.com/GPUOpen-Effects/FidelityFX-FSR2 [F]
[43] https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK [R]
[44] https://gpuopen.com/fidelityfx-super-resolution-4/ [R]
[45] https://github.com/NVIDIA/DLSS ; https://raw.githubusercontent.com/NVIDIA/DLSS/main/LICENSE.txt [R]
[46] https://github.com/NVIDIA-RTX/Streamline [R]
[47] https://github.com/intel/xess ; https://raw.githubusercontent.com/intel/xess/main/LICENSE.txt [R]
[48] https://bevy.org/news/bevy-0-17/ [R]
[49] https://github.com/bevyengine/bevy/tree/main/crates/bevy_anti_alias/src ; https://github.com/bevyengine/bevy/tree/main/crates/bevy_post_process/src [R]
[50] https://research.nvidia.com/publication/2016-06_filtering-distributions-normals-shading-antialiasing [R]
[51] https://www.jp.square-enix.com/tech/publications.html [R]
[52] https://jcgt.org/published/0010/02/02/ (content not verified)
[53] https://google.github.io/filament/main/materials.html [R]
[54] https://research.nvidia.com/publication/2017-02_hashed-alpha-testing [R]
[55] http://diaryofagraphicsprogrammer.blogspot.com/2018/03/triangle-visibility-buffer.html [R]
[56] https://gpuopen.com/fidelityfx-spd/ [F]
[57] https://advances.realtimerendering.com/s2015/index.html — Dreams [R]
[58] https://docs.rs/bevy/latest/bevy/camera/struct.Exposure.html — Bevy `Exposure` [F]
[59] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_core_pipeline/src/tonemapping/mod.rs — Bevy tonemapping source [F]
[60] https://raw.githubusercontent.com/GPUOpen-Effects/FidelityFX-FSR2/master/src/ffx-fsr2-api/shaders/ffx_fsr2_common.h ; https://raw.githubusercontent.com/GPUOpen-Effects/FidelityFX-FSR2/master/src/ffx-fsr2-api/shaders/ffx_fsr2_reproject.h ; https://raw.githubusercontent.com/GPUOpen-Effects/FidelityFX-FSR2/master/src/ffx-fsr2-api/shaders/ffx_fsr2_callbacks_hlsl.h — FSR2 pre-exposure handling [F]
[61] https://raw.githubusercontent.com/GPUOpen-Effects/FidelityFX-FSR/master/ffx-fsr/ffx_fsr1.h — FSR 1 RCAS [F]
[62] https://gpuopen.com/fidelityfx-superresolution/ — FSR 1 [F][V]
[63] https://en.wikipedia.org/wiki/GeForce_30_series — RTX 3060 Laptop / RTX 3060 / RTX 3060 Ti specifications [F]; https://en.wikipedia.org/wiki/GeForce_10_series — GTX 1080 (extract inconsistent, used as a range only)
[64] https://raw.githubusercontent.com/h3r2tic/tony-mc-mapface/main/shader/tony_mc_mapface.hlsl — Tony McMapface LUT dimensions and encoding [F]
[65] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_post_process/src/bloom/mod.rs — Bevy bloom mip sizing [F]
[66] https://github.com/sobotka/AgX ; https://raw.githubusercontent.com/sobotka/AgX/main/config.ocio — AgX configuration [F]

**Tree files read at `6394bc5e`**, all under `D:/wt/docs`:

- **Documents:** `docs/TAA-PLAN.md`, `docs/RENDER-AA-AND-TAILS-PLAN.md`,
  `docs/RENDER-C-AA-DESIGN-PARKED.md`, `docs/RENDER-RUNG3B-TAA-PLAN.md` (headers),
  `docs/OPTIMIZATION-PLAN-RENDER.md` (Track E, X-SPD, C-TSR, §1b P6-S),
  `docs/render/REFLECTIONS-DESIGN-SPACE.md` (R4a, §2.4, §8, §9),
  `docs/render/TRANSPARENCY-DESIGN-SPACE.md` (§2.3, R6, R11, §13.B),
  `docs/RENDER-GRAPH-API-PLAN.md`, `docs/PARTICLES-PLAN.md` (the HDR question),
  `docs/SDF-PERF-AUDIT.md` §1, `docs/SHADER-VARIANT-MANIFEST.md` (format).
- **`boyko_render`:** `aa_config.rs`, `taa_config.rs`, `light.rs`, `render_path_config.rs`,
  `motion_cam.rs`, `instance_model.rs`.
- **`boyko_scene`:** `camera.rs`.
- **`boyko_rhi_vulkan`:**
  - `present/{targets.rs, graph_bridge.rs, frame_driver.rs, gpu_zone.rs, surface.rs}`
  - `framegraph/{graph.rs, sync.rs}`
  - `device.rs`, `rhi_impl/device.rs`, `bindless.rs`, `goldens.rs`
  - `shaders/{pbr_lighting.hlsli, taa_resolve.comp.hlsl, rcas.comp.hlsl, vb_shade.comp.hlsl, gbuffer_mrt.fs.hlsl, vb_geo.comp.hlsl}`
- **`boyko_shaderdsl`:** `lib.rs`, `scalar.rs`, `cf.rs`, `ssao.rs`.
- **`boyko_app`:** `plugins.rs`.

**Added in the third pass**, also at `6394bc5e`:
- **Documents:** `docs/OPTIMIZATION-PLAN-RENDER.md:44`, `docs/VB-SV0-SDF-SHADOW-PLAN.md` (the
  `NMax` lowering), `docs/render/VOLUMETRICS-DESIGN-SPACE.md` (V3, V9, K6) and
  `docs/render/VOLUMETRICS-RESEARCH.md` (its reference-GPU table),
  `docs/render/REFLECTIONS-UPDATE-2026-09-25.md` (RK-16, U7).
- **`boyko_render`:** `taa_state.rs`, `light.rs` (`DirectionalLight`).
- **`boyko_app`:** `runner.rs` (the `mark_reset` site), `tests/forward_mesh.rs`,
  `tests/csm_fit_eval.rs`.
- **`boyko_rhi`:** `encoder.rs` (`dispatch_indirect`).
- **`boyko_rhi_vulkan`:**
  - `present/passes/forward.rs`
  - `rhi_impl/device.rs` (`VkSpecializationInfo`), `Cargo.toml` (`spec_constant_smoke`)
  - `shaders/{taa_resolve.comp.hlsl (full), rcas.comp.hlsl (full), fxaa.fs.hlsl, smaa_common.hlsli, sdf_field.hlsli, particle_draw.fs.hlsl}`
- **`boyko_shaderdsl`:** `interp.rs`, `normal.rs`, `Cargo.toml`.
