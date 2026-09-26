# Volumetrics — the comparative survey (research record)

> Status: research record, 2026-09-25. It covers:
>
> - light shafts (sun rays, god rays);
> - froxel volumetric fog;
> - volumetric clouds;
> - sky and atmosphere, as the lighting context;
> - local volumes (smoke, fire, explosions).
>
> The companion design is
> [`VOLUMETRICS-DESIGN-SPACE.md`](VOLUMETRICS-DESIGN-SPACE.md). It cites this document by section,
> technique number (**T1–T31**) and pitfall number (**P1–P29**).
>
> **Revision 2 (the same day)** follows the design's first critique round (its §12 review log). It
> adds tree facts §0.3a, §0.4a, §0.10a and §0.13; the published T1 cost; the GTX 680 and GTX 1080
> specs; one scaling method (§4.1); Frostbite's apply subtracted from its anchors (§4.2); and
> pitfalls P25–P29. Every repository citation was
> re-opened **by content** at trunk **`6394bc5e`** (worktree `D:/wt/docs`, branch
> `u/research-0925`, identical to the trunk). This directory is not machine-anchored: the gate's
> list is `GATED_DOCS` at `tests/internal_docs_anchors.rs:349`, and it names no `render/`
> document, so the check here was manual. No `cargo` command was run and nothing was timed. Each
> number is either published and cited with its rig, or labelled **[E]** with its derivation.
>
> **Provenance legend.**
>
> | Tag | Meaning |
> |---|---|
> | **[P]** | Primary source read (paper, author's deck, official engine documentation or engine source). |
> | **[T]** | Transcript or text mirror of a primary: a slide transcript or a mirrored paper text. |
> | **[S]** | Secondary: a third-party capture study, blog or spec aggregator. |
> | **[A]** | Abstract or metadata only. |
> | **[U]** | Unverified: seen only in a search snippet or recalled, never read. |
> | **[E]** | This document's own estimate or derivation; the method is stated beside it. |
> | **[Tree]** | Verified in the tree at `6394bc5e`. |
>
> A row marked *(this pass)* was fetched again for this record. Every other web claim comes from
> the research pass that preceded this document, with the URL given in §8.
>
> **Limits.** The session's WebSearch budget (200 calls) ran out partway through the research
> pass. Every source after that point, including every revision-2 source, was reached by fetching
> a known URL.
>
> - **Wronski 2014 was read in full in this pass.** The deck was fetched as a PDF and its text
>   layer extracted with `pypdf`, so its numbers are now **[P]** where the research pass had them
>   as [U].
> - **Transcripts or mirrors only:**
>   - Frostbite 2015 and Schneider 2015 were read through slide transcripts.
>   - Hillaire 2020 was read through a text mirror.
> - **Not read at all:**
>   - The Frostbite 2016 course PDF is larger than the 10 MB fetch limit.
>   - The RDR2 deck is a 232 MB PPTX.

---

## 0. The tree today — what is true before any design (all [Tree])

### 0.1 There is no volumetric rendering

A case-insensitive search of `crates/**` returns two hits, both in `crates/boyko_physics/benches/soft_step_sp2.rs` (lines 69 and 73), where "volumetric" describes a soft-body lattice. The pattern was `fog`, `volumetric`, `god ray`, `light shaft` and `participating medi`. It finds nothing else: no fog pass, no participating-media term, no 3D storage image and no phase function. The designs exist only as plan text:

- [`OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md) Track E:
  - E-FOG `:433`
  - E-SDFVOL `:451`
  - E-DENS-A `:471`
  - E-DENS-B `:489`
  - E-CLOUD `:507`
  - E-GOD `:525`
  - E-PART `:610`
- [`RENDER-AA-AND-TAILS-PLAN.md`](../RENDER-AA-AND-TAILS-PLAN.md)`:109` names volumetrics among the large features deliberately deferred to their own design pass.

### 0.2 The froxel light cull: fixed small grid, VB-only, 8-bit dimensions, 50 m

**Grid and caps** (`crates/boyko_render/src/light.rs`):

| Constant | Value | Line |
|---|---|---|
| `CLUSTER_DIM_X` / `_Y` / `_Z` | 16 / 9 / 24 | `:49` / `:51` / `:53` |
| `MAX_LIGHTS` | 1024 | `:57` |
| `MAX_LIGHTS_PER_CLUSTER` | 256 | `:77` |
| `INDEX_LIST_CAP` | 16384 | `:85` |
| `CLUSTER_NEAR_DEFAULT` | 0.1 | `:843` |
| `CLUSTER_FAR_DEFAULT` | 50.0 | `:848` |

INDEX_LIST_CAP works out to 16384 / 3456 ≈ 4.7 indices per cluster on average. Beyond the far plane (50.0) every pixel clamps to the last slice.

**The config resource.** `ClusterConfig` is a `#[derive(Resource)]` (`light.rs:856`). It carries dims, caps and the exp-Z near/far. Its host packer asserts `dim_x/dim_y/dim_z <= 0xFF` and packs them as `dim_x | dim_y << 8 | dim_z << 16` (`light.rs:956-959`). The shader unpacks the same byte lanes (`light_table.hlsli:357-360`). **Any grid that shares `ClusterParams` is therefore capped at 255 per axis.**

**The slice mapping.** It is `near·(far/near)^(k/dim_z)`:

- **Cull side:** `slice_view_z` (`cluster_cull.hlsl:101-103`), reading `pc.z_near`/`pc.z_far`.
- **Resolve side:** the inverse `cluster_z_slice` (`light_table.hlsli:376`).
- **Both are hand-written HLSL, not eDSL.**
- **The cull never reads depth.** It builds each froxel's world AABB from the four screen-tile corners at the slice's near and far view-z. It keeps lights conservatively: "it can only KEEP a light, never falsely drop one" (`cluster_cull.hlsl:436-456`).

**The cull is armed on VisibilityBuffer only.** The gate is `froxel_light_cull = consumers.clusters_wanted && matches!(path, RenderPath::VisibilityBuffer)` (`crates/boyko_render/src/render_path_config.rs:1252`). It is pinned by the test `froxel_light_cull_is_vb_only` (`:3459`). The transparency design priced arming it elsewhere and did not take it ([`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) D8).

### 0.3 Shadows: surface-shaped functions, one CSM kernel selector

`crates/boyko_rhi_vulkan/shaders/shadow_apply.hlsli` provides three surface-oriented visibility functions:

- `csm_visibility(P, n, view_z, nol)` (`:297`);
- `spot_atlas_visibility(s, P, n, nol)` (`:349`);
- `punctual_atlas_visibility(base, P, n, nol)` (`:396`).

All three take a normal and n·l for the normal and grazing biases. A froxel has neither.

**The CSM PCF is not fixed at 13 taps.** `csm_pcf_disc` (`:192`) selects its kernel through the wave-uniform word `gCsmPcfKernel`:

| Kernel | Constant | Taps |
|---|---|---|
| `TENT13` | 0 | 13 |
| `CROSS5` | 1 | 5 |
| `BILINEAR1` | 2 | 1 |

The constants are at `:151-153`. `BILINEAR1` is one hardware 2×2 comparison, "1 tap, no disc". The file header says the narrower arms are meant for scenes where temporal accumulation supplies the variance. That describes a froxel volume with a history. `atlas_pcf_disc` (`:226`), the spot/point atlas, stays a fixed 13-tap disc: "this kernel selector is CSM-only".

### 0.3a The CSM's reach, and its Y convention (revision 2)

**Past the last split, `csm_visibility` returns fully lit.** Its select counts the splits `view_z` has passed. "Past the LAST active split → no cascade covers the pixel → fully lit (return 1)" (`shadow_apply.hlsli:281`), which the code implements as `if (sel >= gCsmActive)` (`:313`).

**The in-tree reach is 30 m.**

| Fact | Where |
|---|---|
| `DEFAULT_SHADOW_DISTANCE = 30.0`, "the view-space far cap of the last cascade" | `crates/boyko_render/src/csm_config.rs:157` |
| The default `CatchAll` keeps "the last split … at `shadow_distance`" | `:222-223` |
| `Shrink`'s terminator "relocates into the visible scene and jumps up to 29.3% per latch transition" | `:218` |
| `MAX_CASCADES = 4`; the CSM texture is created with `array_layers == MAX_CASCADES` | `csm_config.rs:77`, `texture.rs:120-122` |
| Every in-tree example that arms CSM uses `cascade_count: 3` with that default distance | e.g. `crates/boyko_app/examples/room.rs:29` |

A fog volume reaching 64 m therefore has 34 m of its ray, slices 54–63 of 64, outside every cascade.

**A measured anchor for one more depth pass.** `ZONE_GBUF_CSM_DEPTH` measured 0.067 ms (median, RTX 3060 Laptop, device-local) for a scene of one 160 k-vertex model, and the note records that the cost tracks bytes × passes (`docs/diagnostics/W2208.md:29`) **[M]**. The `csm_depth` pipeline already renders into non-CSM targets: spot shadow faces use it (`present/passes/gbuffer.rs:2360`).

**The file documents its Y convention two ways.**

- The header of the sample function says the lookup "applies the IDENTICAL flip (`uv.y = 1 - (clip.y/clip.w * 0.5 + 0.5)`)" (`shadow_apply.hlsli:114-115`).
- The body does not flip (`uv.y = ndc.y * 0.5 + 0.5`, `:260`). The comment above it warns that a second flip "would Y-flip the READ vs the WRITE, mirroring every shadow", which is invisible only for a caster on the cascade's light-up = 0 line (`:255-259`).

A new lookup written from the header would be mirrored. The tree pins the agreement with a host matrix golden and a host select mirror (`csm_host_select_blend`, `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs:648`).

### 0.4 The SDF field a volume can query

**The gateway.** The analytic gateway is `sdf_field.hlsli`: `field_distance(p)` returns `sdf(p)` (`:246`). It folds at most `MAX_SDF_EDITS = 16u` edits (`:48`), in the loop at `:204-207`. Its Lipschitz constant is `FIELD_LIPSCHITZ_L = 1.41421356` (`:287`).

**The shadow march.** The resolve's shadow march uses:

| Constant | Value |
|---|---|
| `T_MAX` | 10.0 |
| `MAX_IT` | 128u |
| `SHADOW_K` | 8.0 |
| `SHADOW_MINT` | `16.0 * GRAD_H` |

These are set at `deferred_pbr.hlsl:508-511`. The shadow leaf `sdf_soft_shadow_ranged(p, n, L, t_max)` **is eDSL-generated**:

- The sentinels are at `sdf_shadow_leaves.hlsli:49-67`.
- The generic body is `boyko_shaderdsl::shadow::sdf_soft_shadow_ranged_body` (`crates/boyko_shaderdsl/src/shadow.rs:180`).
- The emitter is `emit_hlsl_sdf_soft_shadow_ranged` (`emit/shaders.rs:903`).
- `n` is unused in the body (`_n`).

**The brick atlas is a near-field fixture, not a scene structure.**

- **Image type:** `VK_IMAGE_TYPE_3D` with `TRANSFER_DST | SAMPLED` usage, "NOT storage" (`brick_atlas.rs:79-89`).
- **Format:** `R8Snorm`, falling back to `R16Sfloat` (`:131-132`).
- **Size:** `M2_GRID_DIM = 4` (`compute.rs:4330`) × `BRICK_ALLOC = 10`, so `M2_ATLAS_DIM = 40` voxels per axis (`compute.rs:4340`). That spans [-4, 4]³ at 0.25 m voxels.
- **Levels:** `BRICK_LEVELS = 3` (`crates/boyko_sdf_math/src/brick.rs:195`). Each level's voxel is 2^L wider.

The per-mesh SDF is a separate dense `R8_SNORM` 3D image with `SAMPLED | TRANSFER_DST` usage (`mesh_sdf_texture.rs:91-93`).

### 0.4a What a bound over the edits must know (revision 2)

**The `SdfEdit` layout differs per kind**, and the struct's field comment ("radius (sphere) / half-extents (box)") does not cover the capsule:

| Kind | Layout | Where |
|---|---|---|
| `SPHERE` | `params = (r, 0, 0, 0)` | `crates/boyko_sdf_math/src/lib.rs:150-153` |
| `BOX` | `params.xyz` = half-extents | `lib.rs` `box_shape` |
| `CAPSULE` | `center.xyz` = endpoint a, `params.xyz` = endpoint b, `params.w` = radius | `sdf_field.hlsli:55`, `:84`, dispatched at `:126-133` |

**The field's lower bound.**

- The primitives are exact SDFs.
- The generated `smin` is `lerp(b, a, h) − k·h·(1 − h)` (`sdf_field.hlsli:151-162`), at most `k/4` below the hard min.
- Subtraction and intersection use `smax`/`max`, which never lower the value.
- The fold takes edit 0 as-is (`:210`).

Outside the union of the per-edit AABBs, the field is therefore at least the distance to that union minus `Σ k/4`.

**The shadow leaf's march** (`sdf_shadow_leaves.hlsli`):

- It starts at `t = SHADOW_MINT` from its origin (`:52`).
- It accumulates `res = min(res, SHADOW_K * d / t)` (`:56`).
- It steps `t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP)` (`:60`).

**Two consequences follow.**

- Moving the origin changes the penumbra.
- A ray that misses an object's box by less than `t / SHADOW_K` is still in its penumbra. With the directional march bound `T_MAX = 10` (`deferred_pbr.hlsl:508`, passed at `:1178`) and `SHADOW_K = 8` (`:510`), that reach is 1.25 m.

**The directional SDF shadow is at most 10 m long** on surfaces, for the same reason.

### 0.5 DDGI: world probes, a normal-dependent sampler, off on Forward

- **Grid:** 16 × 8 × 16 = 2048 probes (`ddgi.rs:86`, `:89`, `:92`).
- **Sampler:** `ddgi_probe_sample(p, n, …)` (`ddgi_resolve.hlsli:107`) is hand-written against a host oracle, not eDSL. Its weights include a wrap term `((dot + 1) * 0.5)^2 + bias` built from `n`, which a point in a medium does not have.
- **Forward:** `cap_forward_v1_consumers` (`render_path_config.rs:1379`) switches `ddgi_on` off on Forward/ForwardPlus with `ForwardPreLightConsumersNotYetImplemented`.

### 0.6 `lit` is 8-bit and post-tonemap on every path

- **Format:** `GBUFFER_FORMAT = R8G8B8A8Unorm` (`targets.rs:2279`).
- **Tonemap producers:** `tonemap_select` (`pbr_lighting.hlsli:237`) and `OETF_GAMMA_EXP` (`:202`) run inside **eight** producers:
  - `deferred_pbr.hlsl`
  - `forward_opaque.fs.hlsl`
  - `forward_sky.fs.hlsl`
  - `sdf_forward_march.comp.hlsl`
  - `ssaa_downsample.fs.hlsl`
  - `vb_resolve.comp.hlsl`
  - `vb_shade.comp.hlsl`
  - `vb_shade_split.comp.hlsl`

  That is the same census [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md) R4a records.
- **The designed fix (not landed):** R4a moves `lit` to B10G11R11, falling back to RGBA16F, with one tonemap pass at the tail. It is the same decision as [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) R11 and as [`PARTICLES-PLAN.md`](../PARTICLES-PLAN.md) Open Question 1 (`:2172`).
- **Why fog cares:** fog in-scatter added after the tonemap is added in display space. That is not light transport.

### 0.7 The sky is an analytic two-colour gradient at three sites

- **Forward:** `forward_sky.fs.hlsl`, a gradient plus a sun disc. `SKY_SUN_EXPONENT = 512.0` (`:94`) is a verbatim copy of `deferred_pbr.hlsl`'s constant.
- **Deferred:** the resolve's background branch (`LightHeader H_bg`, `deferred_pbr.hlsl:1497`).
- **VB:** the `vb_sky` pass (`passes/vb.rs:1555`) paints `lit` first. The VB shaders' hemisphere ambient reads `sky_color = L.color; ground_color = L.pos` (`vb_shade.comp.hlsl:520-521`).

There is no transmittance LUT, no aerial perspective and no sun-colour extinction.

### 0.8 Particles exist, as a GPU system, composited into LDR `lit`

- **Shaders:** `particle_emit`, `particle_sim` (and `particle_sim_sdf`, with SDF collision), `particle_sort_*`, `particle_draw`.
- **Generator:** all generator-owned by `boyko_shaderdsl/src/bin/emit_particles.rs`, [`PARTICLES-PLAN.md`](../PARTICLES-PLAN.md) D12. The plan records F13: "the eDSL has no atomics, `groupshared`, stores, or texture sampling" (`:547`).
- **Entities:** particles are not entities; emitters are (D1, `:347`).
- **Lighting:** lit particles evaluate light once per particle in the sim, through a froxel lookup (D11, `:541`).
- **Output:** composited into `lit` with the trade-off "additive clips at white and contributions below 1/255 round to zero" (F2, `:486`).
- **Default off:** on three axes (D13, `:553`); nothing is declared when disarmed.

### 0.9 RHI facts a volume pass meets

- **3D images and storage descriptors exist.**
  - `ImageUsage::STORAGE` (`crates/boyko_rhi/src/enums.rs:457`).
  - A `VK_IMAGE_TYPE_3D` image gets a `VK_IMAGE_VIEW_TYPE_3D` view (`texture.rs:263`).
  - The descriptor vocabulary has `StorageImage` bound by view (`crates/boyko_rhi/src/device.rs:361`).
- **No shader writes a 3D image.** The only `Texture3D` declarations are read-only brick-atlas and mesh-SDF bindings, in `sdf_forward_march.comp.hlsl` and `sdf_gbuffer_composite.hlsl`.
- **Format:** `R16G16B16A16_SFLOAT` storage support is "part of the Vulkan 1.0 CORE mandatory format table" (`targets.rs:2319-2323`, the `GPBR_FORMAT` note).
- **Device limits:** `DeviceCaps` (`device.rs:218`) records `max_image_dimension_2d` (`:392`) and **no 3D limit**.
- **Dispatch indirect:** `cmd_dispatch_indirect` is loaded (`device.rs:599`) and **used** by the particle passes (`passes/particles.rs:514`).
- **Draw indirect count:** `vkCmdDrawIndexedIndirectCount` is deliberately **not** loaded (`device.rs:669-672`).
- **Queues:** there is one queue family with GRAPHICS|COMPUTE (`find_queue_family`, `device.rs:2814`), created with `queue_count: 1` (`:3925`). **There is no async compute.**
- **Frames and semaphores:** the windowed frame driver owns a per-swapchain-image `render_finished` semaphore (`present/frame_driver.rs:48`). `FRAMES_IN_FLIGHT = 2` (`present/mod.rs:92`).
- **GPU timing:** timestamp zones exist (`present/gpu_zone.rs`).

### 0.10 Temporal infrastructure a volume history can reuse

- **Previous camera:** `MotionCamState { prev_view_proj }` is a Resource (`crates/boyko_render/src/motion_cam.rs:101`).
- **Jitter tables:**
  - `JitterSequence::Halton23` (`taa_config.rs:58-62`).
  - The 2D `HALTON_8` table (`taa_jitter.rs:53`).
  - `ign(px, py)` (`deferred_pbr.hlsl:681`).
- **Cross-frame precedent:** `taa_hist` is an RGBA16F ping-pong. It reads `[1-fi]` and writes `[fi]`, is boot-cleared to `GENERAL` and is absent when TAA is off (`targets.rs:468-480`). It is the proof that a cross-frame history works today on the single queue.
- **The second cross-frame history:** the DDGI probe atlas is a "persistent SINGLE atlas per moment — NOT ping-pong" (`crates/boyko_rhi_vulkan/src/ddgi.rs:155-156`). Together with the CSM and atlas images, which are shared by both in-flight frames through `add_image_seeded` (`framegraph/graph.rs:353-360`), it shows that a single image can cross frames on the single queue.

### 0.10a The previous-frame matrix and the camera modes (revision 2)

- **`MotionCamState` carries the marcher-aligned proj·view** ("This frame's marcher-aligned proj·view", `motion_cam.rs:58`).
  - On Deferred that projection has `row2 == row3`, which "pins every billboard vertex to `SV_Position.z == 1.0`" (`docs/SHADER-VARIANT-MANIFEST.md:84`).
  - A reprojection that derives depth from NDC z therefore reads 1 everywhere on that path. The x and y of `clip.xy / clip.w` stay valid.
- **An ORTHO camera mode exists** next to PERSP. The cull converts view-z to the ray parameter by a cosine under PERSP and returns it unchanged under ORTHO (`cluster_cull.hlsl:110-114`).

### 0.11 The depth a fog apply reads

- **Encoding:** `gViewT` is the **ray parameter** t, not view-z: `P = ro + rd * view_t` (`deferred_pbr.hlsl:954-955`).
- **Background sentinel:** `VIEWT_BG = 1.0e30` (`viewt_from_depth.comp.hlsl:58`).
- **Mesh range on Deferred:** `gViewT = md * MESH_DEPTH_T_MAX` with `MESH_DEPTH_T_MAX = 64.0` (`gbuffer_mrt.fs.hlsl:113`). **The Deferred path cannot represent a mesh fragment past 64 m.**
- **Producers per path:**
  - **VB:** `vb_viewt` runs only when a consumer arms it; today that is SSAO (`passes/vb.rs:3911-3919`).
  - **Forward and ForwardPlus:** no producer; that is the reflections campaign's RK-12 ([`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md) §2.2).

### 0.12 The eDSL

- **Leaf shape:** leaves are generic over `C: Cf` and instantiated as `f32` (the host oracle) and as the HLSL printer.
- **Texture fetch is emit-only:** `Node::ResRef` (`crates/boyko_shaderdsl/src/emit/mod.rs:546-552`) exists, and a leaf that uses it has no CPU oracle.
- **Generators:** `src/bin/` holds `emit_field`, `emit_particles`, `emit_probe_gi`, `emit_ssao_variants`, `emit_ui` and `emit_vb`.
- **The rule for skeletons:** a pass skeleton with stores, loops and barriers is owned by a generator as a template with eDSL holes (PARTICLES D12).

### 0.13 Exposure today, and the counters (revision 2)

- **Every lit producer multiplies by `H.exposure`:** `lit = (lit_direct + ambient + emissive) * H.exposure` in the resolve (`deferred_pbr.hlsl:1479`, and `:1481` on the HWRT arm), and the same line in the forward FS (`forward_opaque.fs.hlsl:436`).
- **`LightingConfig::exposure` is the knob:** "the FINAL multiply on accumulated linear radiance. DEFAULT 1.0" (`crates/boyko_render/src/light.rs:562-564`). The same-day POSTFX design makes that field the per-frame pre-exposure, read back from frame N−2 ([`POSTFX-AA-DESIGN-SPACE.md`](POSTFX-AA-DESIGN-SPACE.md) §5.2).
- **Every path reads the same SkyLight row:** `sky_color = L.color; ground_color = L.pos` on Deferred (`deferred_pbr.hlsl:1236-1237`), Forward (`forward_opaque.fs.hlsl:301-302`) and VB (`vb_shade.comp.hlsl:520-521`).
- **Overflow is counted, not logged, in the shipped GPU systems.**
  - `ParticleCounters::clamped_spawns` is a "Diagnostic … Read by the host on a cold path only" (`crates/boyko_render/src/particle.rs:444`).
  - The cull's index-list alloc word takes `InterlockedAdd` of each cell's full demand before the cap check (`cluster_cull.hlsl:376`). Its final value exceeds the cap exactly when the list overflowed.
- **The particle render record's colour lane is RGBA8** (`ParticleRender::color_rgba8`, `particle.rs`), and the particle FS outputs `input.color × texture` (`particle_draw.fs.hlsl`).

---

## 1. Systems surveyed

| System | What it ships for volumetrics | Tag | Source (§8) |
|---|---|---|---|
| Assassin's Creed IV (Wronski 2014) | Frustum-aligned 160×90×64 / 160×90×128 volume; ESM-downsampled sun shadow; around 1.1 ms | [P] *(this pass)* | S1 |
| Frostbite (Hillaire 2015) | Unified froxel V-buffer; energy-conserving integral; 5 % EMA history; volumetric shadow maps; particle voxelization | [T] *(this pass)* | S2 |
| Unreal Engine 4/5 | Volumetric fog (1 ms PS4 High, 3 ms GTX 970 Epic); screen-space Light Shafts (occlusion 0.5 ms at 1080p on a GTX 680); Local Fog Volumes; Sky Atmosphere with directional-light shafts; Volumetric Cloud; Heterogeneous Volumes + SVT | [P] *(fog, light shafts, local fog volumes and sky atmosphere re-read in revision 2)* | S3–S10 |
| Doom Eternal (id Tech 7) | Fixed 160×90×64 froxels; atmosphere LUT amortized over 32 frames; transparents read the volume | [S] *(this pass)* | S11 |
| Godot 4 | Forward+-only volumetric fog; FogVolume shapes; FogMaterial | [P] | S12 |
| Unity HDRP 14 | Volumetric fog with slice-distribution and denoise modes | [P] | S13 |
| Bevy (0.14 → main) | Per-pixel shadow-map raymarch; `VolumetricFog` camera component; `FogVolume` entities; `VolumetricLight` marker | [P] | S14–S16 |
| Horizon Zero Dawn / Forbidden West (Guerrilla) | Nubis 2.5D clouds (about 2 ms PS4); Nubis Evolved; Nubis³ voxel clouds with compressed-SDF march acceleration | [T]/[A] | S17–S19 |
| Hillaire 2020 | LUT sky and atmosphere; aerial-perspective froxel volume; 0.17 ms on GTX 1080 | [T] | S20–S21 |
| Bruneton & Neyret 2008 / 2017 | Precomputed 4D scattering | [P] | S22 |
| NVIDIA | GPU Gems 3 ch. 13 radial blur; Volumetric Lighting SDK (legacy); approximate Mie phase (2023) | [P] | S23–S25 |
| Epipolar family | Engelhardt & Dachsbacher 2010; Chen et al. 2011; Intel and DiligentFX samples | [A]/[P] | S26–S29 |
| NanoVDB / EmberGen | Static-topology sparse volumes; offline flipbook baking | [P] | S30–S31 |

---

## 2. Technique catalogue

### 2.1 Light shafts

**T1 — Screen-space radial blur (Mitchell, GPU Gems 3 ch. 13) [P].**
- **Method:** render occluders black, then march `NUM_SAMPLES` taps per pixel toward the light's screen position. The controls are `exposure`, `weight`, `decay` and `density`.
- **The author's limits:** shafts flicker as occluders cross the screen boundary, and the light's screen position "can tend toward infinity" when the view is perpendicular to it.
- **Cost:** none in Mitchell's chapter. **UE publishes one** (revision 2): occlusion 0.5 ms, and bloom 0.68 ms for a single light, at 1080p on a GTX 680 [P, vendor documentation].
- **UE still ships it** as *Light Shafts*, directional lights only [P], in two modes:
  - an occlusion mode that builds a mask from on-screen depth, blurs it radially away from the light, and uses it to mask fog and atmosphere in-scattering;
  - a bloom mode that is "not really emulating anything that happens in the real world".

  Occlusion shafts survive the sun being "slightly off the screen", but both "fade out as you approach a 90 degree angle from the sun". Because the mask is built from depth, it sees every on-screen caster at any distance, which no shadow-map technique does past its coverage.

**T2 — Epipolar sampling with 1D min/max trees.**
- **Papers:**
  - Engelhardt & Dachsbacher, I3D 2010, DOI 10.1145/1730804.1730823 [A].
  - Chen, Baran, Durand & Jarosz, I3D 2011, DOI 10.1145/1944745.1944752 [A].
- **Implementations:**
  - Intel *Outdoor Light Scattering* (archived) [P]: epipolar slices, 1D min/max binary trees, cascaded shadow maps.
  - DiligentFX `EpipolarLightScattering` [P]: `uiNumEpipolarSlices`, `uiMaxSamplesInSlice`, `fRefinementThreshold`, `bUse1DMinMaxTree`.
- **Constraint:** **one directional light.**
- **Cost:** no timing found in any fetched source.

**T3 — Per-pixel shadow-map ray march (Bevy) [P].**
- **Method:** "raymarching in screen space, transformed into shadow map space for sampling" (0.14 notes), with a Henyey-Greenstein phase.
- **The current module doc:** only "directional lights that have shadow maps"; cost "scales directly with the number of directional lights".
- **Its cost, derived (revision 2):** 1.42–2.8 ms per light at full 1440p × 64 steps, and 0.18–0.35 ms at half resolution × 32 steps, from the reference GPU's listed texture rate (§4.4). Its shafts have shadow-map resolution rather than froxel resolution. It needs the same shadow-map coverage as a froxel volume, so it adds nothing in the far field.
- **ECS surface** (`bevy_light/src/volumetric.rs`):
  - `VolumetricFog` on the camera: `step_count: 64`, `ambient_intensity: 0.1`, `jitter: 0.0`.
  - `FogVolume` entity: `density_factor 0.1`, `absorption 0.3`, `scattering 0.3`, `scattering_asymmetry 0.5`, an optional density texture and offset, `light_tint`, `light_intensity`.
  - `VolumetricLight`, a **unit marker component** on a light: the structural-capability pattern.
- **NVIDIA's legacy Volumetric Lighting SDK** [P] extrudes volumes from shadow-map data for directional, omni and spot lights, and shipped in Fallout 4. The page gives no cost.

**T4 — Shafts as shadowed in-scatter in a froxel volume** (T5–T8).
- **Method:** each froxel stores in-scattered light computed with shadow-map visibility, so integrating along the view ray produces shafts from every shadowed light, on-screen or not.
- **Wronski's own framing:** "if we added simple shadowing term, we would gain light shafts/god rays just for the cost of calculating the shadowing" (S1, p. 17 text).
- **Why T4 subsumes T1–T3 inside its range:** T1 is sun-only, screen-bound and breaks at 90°. T2 is one directional light. T3 costs per pixel × step × light. T4 costs per froxel, independent of screen resolution, and serves any number of lights and transparent layers (S1 p. 44: "applied … on both solid (1) and multiple transparent objects (2) and (3)").
- **Where it does not (revision 2):** T4 sees a caster only where a shadow source covers the froxel and only out to the volume's far plane. Past that, UE combines volumetric fog with T1's occlusion mode (T1) and with atmosphere shafts through a very large shadow distance (T24). "Converged on froxels" is true of the near and mid field only.

### 2.2 Froxel volumetric fog

**T5 — Assassin's Creed IV (Wronski, SIGGRAPH 2014) [P] *(this pass, `pypdf` text of the deck)*.**
- **Layout:** "aligned with the camera frustum … normalized device coordinates in width x height axis and for depth slices we use exponential depth distribution". The distribution was "concentrated near the camera".
- **Grid:** "We used volumes sized 160x90x64 or 160x90x128 depending on the platform. It provides fixed cost of almost all of the passes, not dependent on the screen resolution."
- **Range:** "distances between 50 and 128 meters".
- **Sun shadow:** "we downsample our cascaded shadowmaps four times (target R32F 1024 x 256 texture)" into an **Exponential Shadow Map** with a separable box blur, because the full cascades (4 × 1536² or 1k²) flickered in fog. ESM's leaking "wasn't noticeable … in participating media".
- **Media and lighting:** density (one octave of wind-animated Perlin noise plus a vertical exponential falloff) and lighting are **fused in one pass**, "due to a bit smaller bandwidth usage". "They can be split and totally decoupled." Lights are the sun, a constant ambient, and point lights "marked by artists as lights affecting the atmosphere". The last is the per-light opt-in.
- **Integration:** "a 2D compute shader thread group marches through 3D volume, accumulating in-scattered lighting and extinction".
- **Apply:** "just a simple bilinear look-up from a 3D texture and a fused multiply-add", in forward or deferred, for any number of transparent layers.
- **Cost:** "Total cost was surprisingly small, around 1.1ms. Calculating it in double resolution had a cost of 1.6ms." Building density plus lighting was "around 0.43ms"; the other passes were "all under 0.2ms". The deck's text layer does not name the console on that slide; the SIGGRAPH abstract names PS4 and Xbox One.
- **Extensions after AC4 (same deck):**
  - SH-based sky/GI ambient in fog ("Lack of support of fog ambient/sky lighting results in too much scene darkening").
  - An HG-to-zonal-SH phase.
  - Temporal jitter plus reprojection: "reprojection is much easier in 3D than in 2D" because data behind a moving object stays valid.
  - HZB early-out and clustered light culling for fog.
  - Async compute ("as soon as shadowmaps are ready").
  - Density from particles or analytic shapes.

**T6 — Frostbite (Hillaire, SIGGRAPH 2015) [T] *(this pass, slide transcript)*.**
- **V-buffer:** two RGBA16F volumes, scattering RGB + extinction and emissive RGB + phase g, over σa, σs, g and emissive.
- **Grid:** 8×8-pixel tiles by default, 4×4 optional, 64 slices. It reuses the **16×16** tiled light cull, a *coarser* list than the fog grid (research pass).
- **Energy-conserving integral:** `∫₀ᴰ e^(−σt·x)·S dx = (S − S·e^(−σt·D))/σt`. It replaces the single-sample form that "caused light leaking".
- **Temporal:** Halton jitter with the same offset along each view ray; "5% Blend current with previous"; history is dropped outside the frustum.
- **PS4 at 900p, 64 slices:**

  | Pass | 8×8 tiles | 16×16 tiles |
  |---|---|---|
  | Material voxelization | 0.45 ms | 0.15 ms |
  | Light scattering | 2.00 ms (local lights 1.1 + sun +0.5 + temporal +0.4) | 0.50 ms |
  | Final accumulation | 0.40 ms | 0.08 ms |
  | Apply | +0.1 ms | +0.1 ms |
  | **Total** | **2.95 ms** | **0.83 ms** |

- **Volumetric shadow maps:** 32³, ray-marched at 0.04 ms for a spot light and 0.14 ms for a point light.
- **Particle voxelization:** 1k particles cost 0.03 ms (default) or 0.25 ms (high quality).
- **Memory (research pass):** "160x90x64 (~7mb per rgbaF16 texture)" for 720p, and 240×135×64 (about 15 MB) for 1080p.
- **Limits:**
  - Material animation leaves reprojection trails.
  - Moving lights cause artifacts.
  - The phase function is a single lobe.
  - "Scaling to 4K screens remains open challenge".

**T7 — Unreal Engine volumetric fog [P] *(re-read this pass)*.**
- **Cost:** "Volumetric Fog costs 1 millisecond on PlayStation 4 at High settings, and 3 milliseconds on an NVIDIA 970 GTX on Epic settings, which has eight times more voxels to operate on."
- **Temporal:** "a heavy temporal reprojection filter with a different sub-voxel jitter per frame … fast-changing lights, like flashlights and muzzle flashes, leave lighting trails".
- **Lights:**
  - one directional light, "shadowing from Cascaded Shadow Maps or static shadowing". The page says nothing about froxels past the dynamic shadow distance (re-read in revision 2);
  - "any number of Point and Spot Lights". Shadow-casting ones cost "approximately three times more than unshadowed";
  - sky light shadowing. The research pass read DFAO as the movable sky light's occluder. The re-fetch summary named only "Shadowing of Stationary Sky Lights", so the DFAO detail stands on the research pass alone.
- **Unsupported:** IES profiles, light functions on point/spot lights, ray-traced shadows.
- **UE4 scalability presets** (`BaseScalability.ini` copy, research pass):

  | Preset | `GridPixelSize` | `GridSizeZ` | Other |
  |---|---|---|---|
  | High | 16 | 64 | — |
  | Epic | 8 | 128 | — |
  | Cine | 4 | 128 | `HistoryMissSupersampleCount=16` |

  **The "eight times" is consistent with these presets at 1080p [E].** High is 120×68×64 = 522,240 froxels; Epic is 240×135×128 = 4,147,200; the ratio is 7.94.

**T8 — Doom Eternal (Coenen's RenderDoc study) [S] *(this pass)*.**
- **Grid:** "The 3D textures are 160x90x64 which means froxels of 12x12 pixels in size": a fixed grid at 1080p.
- **Passes:** four — atmosphere LUT, per-froxel scattering, propagation toward the viewer, composite.
- **Amortization:** the sky data is "amortized over 32 frames".
- **Transparents:** "The light scattering data is also used to get good looking scattering inside the surface."

**T9 — Godot 4 [P].**
- Forward+ only; volume size and depth; "Use Filter"; a temporal reprojection amount, where higher values cause "ghosting and leaving a trail".
- FogVolume shapes: Ellipsoid, Cone, Cylinder, Box, World.
- FogMaterial: density (±), albedo, emission, height falloff, edge fade, density texture.
- Per-light volumetric fog energy.
- Known issues: "Thin fog volumes may appear to flicker when the camera moves"; "banding … especially at higher density levels".

**T10 — Unity HDRP 14 [P].** Screen-resolution percentage; slice count; slice-distribution uniformity (0 = exponential, 1 = uniform); anisotropy −1..1; denoising modes None, Reprojection ("static lighting"), Gaussian ("dynamic lighting") and Both.

**T11 — Bevy fog volumes** (see T3). The camera component plus `FogVolume` entities is the closest ECS precedent for local media as entities.

**T12 — Analytic exponential height fog (Quilez) [P].** Closed form `(a/b)·exp(−ro.y·b)·(1−exp(−t·rd.y·b))/rd.y`, "no more than one division". It is the standard far-field companion beyond the froxel range.

**T13 — UE Local Fog Volumes [P].** Analytic spheres, culled with screen tiles; no shadowing. They render after height fog and before volumetric fog. The cap is `r.LocalFogVolume.TileMaxInstanceCount`, default 32, a maximum "per view (and per tile for consistency)". That is a per-view cap first (wording re-read in revision 2).

**T14 — Phase functions.**
- **Jendersie & d'Eon (NVIDIA, SIGGRAPH 2023 talk) [P]:** "a blend of HG and Draine's phase function can accurately match 95% of the Mie phase function over a wide range of droplet sizes". It is analytic, with no tables, and costs "roughly equal time" to simpler fits. HG and Cornette-Shanks leave "a lack of accuracy for fog, clouds, skies".
- **AC4 (S1):** no physical phase; two art colours (toward and away from the sun).
- **Frostbite:** a single HG lobe.

**T15 — Temporal reprojection of the volume.**
- AC4, after shipping: 1-sample jitter; "almost all edge artifacts are gone".
- Frostbite: 5 % EMA.
- UE: per-frame sub-voxel jitter.
- Godot and HDRP: an amount or mode.
- **In 3D, history behind a moving occluder stays valid** (S1 p. 58). Only data that leaves the frustum is invalid.

### 2.3 Volumetric clouds

**T16 — Horizon Zero Dawn Nubis (Schneider & Vos, Advances 2015) [T].**
- **Noise:** a 128³ RGBA Perlin-Worley + Worley texture, a 32³ RGB Worley detail texture, and a 128² curl texture.
- **Samples:** 64 per ray, "a potential 128 at the horizon".
- **Light:** 6 cone samples toward the sun plus one far sample.
- **Temporal:** "a quarter res buffer to update 1 out of 16 pixels for each 4x4 pixel block", which made the shader "10x faster". Cost went from **20 ms to about 2 ms on PS4**.
- **Memory:** the sky system is "20 mb of ram".

**T17 — Nubis, Evolved (Forbidden West, SIGGRAPH 2022) [A].** Fly-through cloudscapes at "1080p resolution without the use of temporal upscaling"; "near zero cost" internal lighting.

**T18 — Nubis³ (Guerrilla 2023) [A].** Moves from "2.5D methods" to voxel clouds. Ray-march acceleration uses **"compressed signed distance fields"**, the closest shipped precedent for an SDF-native volumetric advantage. There are no numbers on the page.

**T19 — UE Volumetric Cloud [P].**
- `r.VolumetricRenderTarget` modes: 0 = quarter-res trace with half-res reconstruction; 1 = half-res trace; 2 = fast, but "does not support cloud intersection with opaque meshes".
- Cloud shadows: ray-marched, or cascaded "Beer shadow maps".
- Sky-light cloud AO. No ray tracing.

**T20 — RDR2 (Bauer, Advances 2019) [A].** Voxelization plus ray-marching for scattering and transmittance, with sky irradiance probes. No numbers were retrieved; the deck is a 232 MB PPTX.

**T21 — Frostbite 2016 physically based sky, atmosphere and clouds [A].** The course PDF exceeds the fetch limit; no numbers.

### 2.4 Sky and atmosphere — the lighting context

**T22 — Bruneton & Neyret 2008** (CGF, DOI 10.1111/j.1467-8659.2008.01245.x) [A], with the 2017 reference re-implementation [P]:
- a 4D scattering LUT packed into 3D;
- the horizon artifact removed;
- ozone and custom density profiles;
- light shafts implemented only partly ("but not the shadow volume algorithm").

**T23 — Hillaire 2020** (EGSR/CGF, DOI 10.1111/cgf.14050) [T]:

| LUT | Size | Steps | GTX 1080 | iPhone 6s |
|---|---|---|---|---|
| Transmittance | 256×64 | 40 | 0.01 ms | 0.53 ms |
| Sky-view | 200×100 | 30 | 0.05 ms | 0.27 ms (at 96×50) |
| Aerial perspective | 32³ | 30 | 0.04 ms | 0.11 ms (at 32²×16) |
| Multiple scattering | 32² | 20 | 0.07 ms | 0.12 ms |

- The aerial-perspective volume is "32 × 32 over the screen and 32 depth slices over a depth range of 32 kilometers".
- **Multiple-scattering assumptions:** orders ≥ 2 use an isotropic phase; the neighbourhood is uniformly lit; visibility is ignored.
- Hue drifts at "very high scattering coefficients".
- **Volumetric shadows in the atmosphere** (revision 2, the same mirror). The paper names shadowing from mountains onto the atmosphere as an important effect to reproduce.
  - The paper rules out epipolar sampling (the atmosphere is not homogeneous) and shadow volumes (the LUTs do not allow that integral).
  - It recommends "per-ray sample jittering and reprojection". That is a per-pixel march, the T3 class.
  - It publishes no cost for it.
- The reference code `sebh/UnrealEngineSkyAtmosphere` is **MIT-licensed** [P].

**T24 — UE Sky Atmosphere [P].** Aerial perspective is applied to opaque and translucent surfaces through the froxel volume. The sky material is excluded "to avoid double contribution".
- **Shafts in the atmosphere** (revision 2): the directional light's shadowing produces sunlight shafts in the atmosphere, for ground-level and space views, enabled by the light's *Cast Shadow on Atmosphere*.
- **Its shadow-distance recommendation:** a high Dynamic Shadow Distance; its examples use 1000 km. UE's far-field shafts therefore rest on a very long shadow coverage, which is the premise of the design's D21.

### 2.5 Local volumes — smoke, fire, explosions

**T25 — Particles injected into the fog volume.** Frostbite voxelizes particles (1k particles for 0.03 ms, T6). UE uses Volume-domain particle materials (albedo, emissive, extinction) [P]. AC4 lists particle and analytic-shape injection as the extension path (S1 p. 67). **Resolution is bounded by the froxel size.**

**T26 — UE Heterogeneous Volumes, "Experimental" in 5.8 [P], fed by Sparse Volume Textures [P].**
- Ray-marched single scattering with ray-marched shadows, and a lighting cache capped at "1024 x 1024 x512".
- SVT: a page table plus physical tiles; up to 2 data textures (8 channels); 8/16/32-bit; page table ≤ 2 GB. Streaming degrades above "30 - 50 megabytes (MB)" per frame.

**T27 — NanoVDB [P].** "static-topology": "While values can be modified … its tree topology cannot". Pointer-less and contiguous; tested with Vulkan, HLSL and GLSL through `PNanoVDB.h`.

**T28 — Offline-simulated flipbooks (EmberGen) [P].** "game-ready, fully assembled flipbooks", motion vectors, 6-point lighting and VDB export. An offline tool: the runtime is a textured particle.

### 2.6 What distance fields do in shipped volumetrics

**T29 — UE sky-light occlusion in fog through DFAO** (research pass [P]; see T7's caveat). This is an SDF consumer inside a fog pipeline, shipped.

**T30 — Nubis³ SDF-accelerated cloud march** (T18). Empty-space skipping for a density grid.

**T31 — SDF soft shadows (Quilez) [P].** The penumbra estimate `res = min(res, k·d/t)` is the body the engine's `sdf_soft_shadow_ranged` already generates (§0.4).

**No shipped primary source was found for SDF-derived *density* in games.** E-DENS-B in the render plan would be a first, not a reproduction.

---

## 3. Comparative table

Every cost is someone else's GPU (§4 normalizes them). "HYBRID fit" asks what the technique does with **SDF casters**, which are not in any shadow map in this engine (§0.4).

| # | Technique | Lights served | Resolution scaling | Published cost | SDF casters | Transparents | In-house? |
|---|---|---|---|---|---|---|---|
| T1 | Radial blur | sun, on-screen only | ∝ pixels × taps | UE occlusion 0.5 ms, 1080p, GTX 680 | a depth-derived mask sees SDF and mesh casters alike, at any distance | no | trivially |
| T2 | Epipolar + min/max | one directional | ∝ slices × samples | none found | only via a shadow map | no | yes (M–L) |
| T3 | Per-pixel SM march | directional (Bevy) | ∝ pixels × steps × lights | none published; derived in §4.4 | only via a shadow map | no | yes |
| T4–T8 | Froxel fog | all, shadowed | **fixed grid: none**; pixel-tiled: ∝ pixels | AC4 about 1.1 ms; Frostbite 2.95 / 0.83 ms PS4; UE 1 ms PS4, 3 ms GTX 970 | a per-froxel field query (§4.3) | one fetch per fragment | yes |
| T12–T13 | Analytic height fog and local volumes | none | ∝ pixels, a few ALU | none (trivial) | n/a | yes | trivially |
| T16–T19 | 2.5D clouds | sun + ambient | ∝ pixels (quarter-res + 1/16 update) | about 2 ms PS4 | n/a | via composite order | yes (XL) |
| T18 | Voxel clouds + SDF accel | sun | ∝ pixels × steps | none | the field **is** the accelerator | — | yes (XL, research) |
| T23 | LUT sky | sun + sky | tiny fixed LUTs | 0.17 ms GTX 1080 | n/a | aerial perspective on translucents | yes; MIT reference |
| T25 | Particle injection | as fog | ∝ particles | 1k particles 0.03 ms PS4 | n/a | inherits fog | yes |
| T26–T27 | Heterogeneous volumes | as volume | ∝ pixels × steps × shadow steps | none | the field can skip empty space | own composite | NanoVDB would be third-party |
| T28 | Flipbooks | particle lighting | particle fill | particle cost | n/a | yes | tool offline, runtime in-house |

---

## 4. Cost anchors — normalized per froxel, then scaled to the reference GPU

### 4.1 The reference GPU and the anchors' GPUs

Figures are vendor specifications as listed by Wikipedia *(this pass)*; they are not measurements.

| GPU | FP32 | Bandwidth | Texture rate | Source |
|---|---|---|---|---|
| **RTX 3060 Laptop** (the tree's reference, `OPTIMIZATION-PLAN-RENDER.md` §0) | "6.912 (10.94)" TFLOPS | "336 (288)" GB/s | "166.4 (204.4)" GT/s *(revision 2)* | Wikipedia, GeForce 30 series |
| GTX 970 | 3,920.3 GFLOPS | 196.3 GB/s (3.5 GB segment) | not fetched | Wikipedia, GeForce 900 series |
| PS4 | 1.84 TFLOPS | 176.0 GB/s | not fetched | Wikipedia, PS4 technical specifications |
| GTX 680 *(revision 2, UE's Light Shafts rig)* | 3,090.43 GFLOPS | 192.256 GB/s | 128.8 GT/s | Wikipedia, GeForce 600 series |
| GTX 1080 *(revision 2, Hillaire 2020's rig)* | "8228 (8873)" GFLOPS | 320 GB/s | 352 GT/s | Wikipedia, GeForce 10 series |

The RTX 3060 Laptop figures depend on the laptop's power configuration; each listed pair is the table's own.

Scale ratios for the RTX 3060 Laptop **[E]**:

| From | Bandwidth ratio | FLOPS ratio | Texture-rate ratio |
|---|---|---|---|
| PS4 | 336 / 176 = **1.91×** | 3.76–5.95× | — |
| GTX 970 | 336 / 196.3 = **1.71×** | 1.76–2.79× | — |
| GTX 680 | 336 / 192.256 = 1.75× | 2.24–3.54× | 166.4 / 128.8 = **1.29×** |
| GTX 1080 | 336 / 320 = 1.05× | **0.84**–1.23× | 0.47× |

**One scaling method (revision 2).**

- Each anchor is scaled by every ratio that can bind the technique.
- The **smallest applicable** ratio gives the conservative end. The design's gates and budgets use that end.
- The largest gives the optimistic end.

**What binds each technique:**

- Fog passes stream 3D textures and loop over lights. Bandwidth binds them, and its ratio is the smallest applicable one for the PS4 and GTX 970 anchors.
- Cloud marching is dominated by 3D-noise fetches, so bandwidth binds it too.
- A radial blur is a cache-friendly tap loop, so the texture rate binds it.
- LUT generation is ALU work with few fetches, so the FLOPS ratio binds it.

Revision 1 scaled clouds by FLOPS alone, which was the optimistic end.

### 4.2 Per-froxel cost **[E]**

The grids below are derived, and the time per froxel is the published time divided by the froxel count.

| Anchor | Grid | Froxels | Published | ns/froxel (native) | ns/froxel (3060 L, ÷ BW ratio) |
|---|---|---|---|---|---|
| AC4 (console) | 160×90×64 | 921,600 | about 1.1 ms | 1.19 | **0.62** |
| AC4 "double resolution" (read as 160×90×128) | 160×90×128 | 1,843,200 | 1.6 ms | 0.87 | 0.45 |
| Frostbite PS4 900p, 8×8, apply subtracted (2.95 − 0.1) | 200×113×64 | 1,446,400 | 2.85 ms | 1.97 | **1.03** |
| Frostbite PS4 900p, 16×16, apply subtracted (0.83 − 0.1) | 100×57×64 | 364,800 | 0.73 ms | 2.00 | 1.05 |
| Frostbite 8×8 without local lights (2.85 − 1.1) | same | 1,446,400 | 1.75 ms | 1.21 | 0.63 |
| UE PS4 High (1080p assumed) | 120×68×64 | 522,240 | 1 ms | 1.91 | 1.00 |
| UE GTX 970 Epic (1080p assumed) | 240×135×128 | 4,147,200 | 3 ms | 0.72 | **0.42** |

**The anchor range for the reference GPU is 0.42–1.03 ns per froxel for the whole chain, apply excluded [E].**

- **Revision 1's 1.07 was wrong.** Frostbite's 2.95 / 0.83 ms totals are the sum of its rows including the +0.1 ms apply (0.45 + 2.00 + 0.40 + 0.1 = 2.95; 0.15 + 0.50 + 0.08 + 0.1 = 0.83), so the apply is now subtracted.
- **Whether AC4's "around 1.1ms" and UE's totals include their apply is not stated.** Those rows are kept as published; including an apply would only overstate their per-froxel cost.
- **What the ends are.** The top end carries Frostbite's many local lights; the bottom end is UE at high occupancy.
- **Small grids do not scale down linearly.** Frostbite's 16×16 row is per-froxel *dearer* than 8×8, which is the fixed per-dispatch overhead showing.

### 4.3 Grids against resolution **[E]**

Costs use 0.42–1.03 ns per froxel. VRAM counts **three** RGBA16F volumes: a scatter ping-pong, which doubles as the history, and one integrated volume shared by both frames in flight through a seeded barrier (the CSM / DDGI-atlas precedent, §0.10). The design justifies that chain (DESIGN §4.2); revision 1 counted four.

| Grid | Froxels | Output-resolution dependence | Inject + integrate | 3 × RGBA16F |
|---|---|---|---|---|
| 160×90×64, fixed (AC4, Doom Eternal) | 0.92 M | none | **0.39–0.95 ms** | **22.1 MB** |
| 160×90×128, fixed (AC4 high) | 1.84 M | none | 0.77–1.90 ms | 44.2 MB |
| 8-px tiles × 64 @ 1080p (240×135) | 2.07 M | ∝ pixels | 0.87–2.14 ms | 49.8 MB |
| 8-px tiles × 64 @ 1440p (320×180) | 3.69 M | ∝ pixels | 1.55–3.80 ms | 88.5 MB |
| 8-px tiles × 64 @ 4K (480×270) | 8.29 M | ∝ pixels | 3.48–8.54 ms | 199 MB |

The apply pass is the only per-pixel part. Frostbite measured +0.1 ms at 900p on PS4, which is 1.44 M pixels at 176 GB/s. That is about 69 ps per pixel, and about 36 ps per pixel after bandwidth scaling. A bandwidth bound of 16 B per pixel at 336 GB/s gives the upper figure. **[E]:**

| Resolution | Pixels | Apply |
|---|---|---|
| 1080p | 2.07 M | 0.07–0.10 ms |
| 1440p | 3.69 M | 0.13–0.18 ms |
| 4K | 8.29 M | 0.30–0.40 ms |

### 4.4 Other anchors, scaled **[E]**

| Item | Derivation | 3060 Laptop @ 1440p |
|---|---|---|
| T23 sky LUTs | 0.17 ms on GTX 1080, ALU-bound: ÷ 0.84 (conservative FLOPS) to ÷ 1.23 | **0.14–0.20 ms**, less if amortized as in Doom Eternal |
| T16 2.5D clouds | 2 ms on PS4 at 1080p × 1.78 pixel ratio; fetch-bound, so ÷ 1.91 (bandwidth, conservative) to ÷ 5.95 (FLOPS, optimistic) | **0.60–1.86 ms** (1080p 0.34–1.05, 4K 1.34–4.19); noise 8.5 MB (128³ + 32³ RGBA8, 128² curl). Revision 1's 0.6–0.95 was the FLOPS end only |
| T1 radial blur, occlusion mode | UE's published 0.5 ms at 1080p on a GTX 680 × 1.78 pixel ratio; tap-bound, so ÷ 1.29 (texture rate, conservative) to ÷ 3.54 (FLOPS, optimistic) | **0.25–0.69 ms** (1080p 0.14–0.39, 4K 0.56–1.55), sun only |
| T3 per-pixel march, full resolution | 3.69 M px × 64 steps = 236 M comparison taps per directional light ÷ 166.4 GT/s = a 1.42 ms floor; 2× realistic | 1.42–2.8 ms per light |
| T3 per-pixel march, half resolution × 32 steps | 0.92 M px × 32 = 29.5 M taps ÷ 166.4 GT/s = a 0.18 ms floor; 2× realistic. No fetched primary source pins a shipped half-resolution configuration | 0.18–0.35 ms per light, with a bilateral upsample on top |
| One more depth-only cascade | the tree's whole CSM depth zone measured 0.067 ms for one 160 k-vertex model [M] (§0.3a) | ≤ 0.067 ms for the same casters; ∝ caster bytes × passes |
| SDF-only light-space map, 128²–256² | texels × 24 steps × 16 edits × 25 FLOP = 0.16–0.63 GFLOP at 50 % of 6.9 TFLOPS | 0.05–0.18 ms per rebuild |

---

## 5. Pitfalls register

| # | Pitfall | Who reports it | Consequence for this engine |
|---|---|---|---|
| P1 | Temporal trails from fast-changing lights and animated media | UE (T7), Frostbite (T6), Godot (T9) | A history-off mode is needed for goldens and a trail bound for flashing lights |
| P2 | Thin volumes flicker as the camera moves; banding at high density | Godot (T9) | Jitter + history; an edge-fade term on local volumes |
| P3 | Light leaking from single-sample integration | Frostbite (T6) | Use the energy-conserving integral (T6) |
| P4 | Undersampling when the range grows | UE "View Distance" (T7) | Fixed exp-Z grid plus an analytic tail (T12) beyond it |
| P5 | Screen-space shafts break at screen edges and at 90° | GPU Gems 3, UE (T1) | Do not build T1 as the shaft mechanism |
| P6 | Cost ∝ froxel count; 4K is "open" | Frostbite (T6) | A fixed grid decouples cost from output resolution (T5, T8) |
| P7 | Shadowed local lights cost about 3× unshadowed | UE (T7) | Per-light opt-in; cheapest shadow tap per froxel |
| P8 | Full-resolution cascades flicker in fog | AC4 (T5) | Low-pass the sun shadow for fog: a 1-tap kernel + history, or an ESM downsample |
| P9 | HG / Cornette-Shanks error for fog, clouds and sky | Jendersie & d'Eon (T14) | HG first; an HG+Draine leaf later |
| P10 | Fast cloud motion breaks temporal upscaling; the fastest UE mode cannot intersect opaques | Nubis Evolved (T17), UE (T19) | Cloud rung carries a motion bound |
| P11 | Sky double-counts aerial perspective | UE (T24) | Exclude sky pixels from the AP apply |
| P12 | Transparents must read the volume, not be fogged by the opaque apply | AC4 (T5), Doom Eternal (T8), UE (T24); `TRANSPARENCY-RESEARCH.md:91-92` records the orderings | Per-fragment fetch in translucent shaders; per-particle for particles |
| P13 | Sparse volume topology is static (NanoVDB); SVT streaming degrades above 30–50 MB/frame | T26, T27 | Local volumes as dense per-volume grids first |
| P14 | Temporal reprojection is non-deterministic | general | Byte goldens only with history off; a statistical bar with history on |
| P15 | Fog applied after the tonemap is not light transport | §0.6 | Fog needs the HDR scene colour (R4a / R11) |
| P16 | Fog without ambient in-scatter over-darkens shadowed regions | AC4 (T5, p. 52) | Ambient term from day one (hemisphere; DDGI where armed) |
| P17 | The render plan's grid equality "fog grid == cluster grid" is unsatisfiable today | §0.2 | See §6 |
| P18 | The render plan's VRAM row mis-multiplies | §6 | See §6 |
| P19 | A per-froxel analytic SDF march at full grid resolution is O(froxels × steps × edits) | E-SDFVOL (`OPTIMIZATION-PLAN-RENDER.md:461`) | Bound it: half resolution per axis + a per-kind, penumbra-inflated edit-bounds prefilter (DESIGN §3.3); valid to 16 edits, P9 beyond |
| P20 | `maxImageDimension3D` is not recorded by the device layer | §0.9 | A new `DeviceCaps` field and a boot degrade |
| P21 | Forward/ForwardPlus have no `gViewT` producer; VB produces it only when armed | §0.11 | Fog arms the producer; Forward waits on RK-12 |
| P22 | The Deferred path cannot represent mesh depth past 64 m | §0.11 | The fog range default follows it |
| P23 | fp16 radiance in the volumes can overflow once the sky is physical (large sun radiance) | T23 by implication; Frostbite/UE pre-expose | Pre-exposure at injection **from the first fog rung**, not from the sky rung, because surfaces are already exposed (§0.13) |
| P24 | DDGI's sampler is surface-shaped (wrap term on `n`) | §0.5 | A normal-free sibling for the isotropic ambient |
| P25 | Froxel-scale light leaking. **Depth:** a trilinear fetch at the pixel's slice coordinate adds up to half a slice of in-scatter from behind the surface. **XY:** the filter blends a neighbouring column across a depth edge | revision 2 (critique O7). Wronski's HZB early-out is presented as a *performance* measure (S1 p. 49), and the deck notes that such culling voids the 3D-history property (p. 58) | Sample at the slice's far edge (a half-texel shift); accept the XY half, with a depth-aware XY weight as the escape. Do not take the HZB early-out with a 3D history |
| P26 | A froxel past the CSM's last split is lit, because the surface lookup returns 1 there | §0.3a | A fog-owned far cascade over `[last split, z_far]` (DESIGN D21) |
| P27 | An α = 0.05 EMA stored in fp16 stalls up to 0.5 ULP / 0.05 = 10 ULP from its target, ≈ 0.5–1 % relative | arithmetic (critique W2) | State the bias; gate the history against the EMA recursion itself, not a uniform mean |
| P28 | A pre-exposed history blended across an exposure change mixes two scales | POSTFX §5.2's invariant; critique W1 | Rescale the history by `pre_N / pre_{N−1}` at reprojection |
| P29 | A soft-shadow leaf's penumbra extends `t / k` beyond its casters' bounds, so a ray-vs-AABB prefilter on tight bounds misses shadowed cells | §0.4a | Inflate the bound by `T_MAX / SHADOW_K` (1.25 m here) and by `Σ smoothness / 4` |

---

## 6. Plan inputs that no longer match the tree (the delta facts)

These are statements in [`OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md) Track E, checked against `6394bc5e`.

1. **"the froxel XY×Z IS the cluster grid … fog grid dimensions to match P7's froxel grid"** (E-FOG, `:433`) at "160×90×64".
   - The tree's grid is 16×9×24 over 0.1–50 m (§0.2).
   - It runs on VB only.
   - It is packed 8 bits per axis.
   - 64 is not an integer multiple of 24.

   The equality cannot hold as written.
2. **"needs P1+P6-S+P7"**, where P6-S is "the `submit_windowed` semaphore-present seam" (`:149-157`).
   - The windowed frame driver owns `render_finished` semaphores (§0.9), and two cross-frame histories run today (§0.10).
   - `crates/boyko_rhi_vulkan/src/queue.rs`, which the plan cites for the seam, does not exist at `6394bc5e`.

   P6-S is not a live prerequisite.
3. **The VRAM row** "Froxel/fog 3D textures (E-FOG 160×90×64 ×RGBA16 ×2) | ~30 MB" (`:853`). Two RGBA16F volumes at that size are 2 × 7.37 = **14.7 MB**. About 30 MB is four volumes. The design's revision-2 chain totals ≈ 27.0 MB at Low: three volumes, the SDF visibility volume, the fog far cascade and the light tables.
4. **E-SDFVOL "P9-REQUIRED above the ≤16-edit fixture"** (`:461`). The analytic gateway is still hard-capped at 16 edits (§0.4), so "above 16" is not reachable through it today. P9's brick atlas exists, but as a 40³ SAMPLED near-field image, not a scene structure. The cost concern stands; its trigger has not fired.
5. **E-DENS-A "a second R16 brick-atlas channel"** (`:471`). The atlas is `TRANSFER_DST | SAMPLED`, single-channel `R8Snorm` / `R16Sfloat`, 40³ (§0.4). A density channel there covers [-4, 4]³ only.
6. **E-GOD route (a) "FREE as a byproduct of E-SDFVOL"** (`:525`). Only for SDF casters. Mesh casters reach fog only through CSM or the atlas (§0.3), whose functions are surface-shaped and need a normal-free form.
7. **Sky and atmosphere are absent from Track E.** Aerial perspective and sun transmittance are the lighting context for fog and clouds (T23, T24).

---

## 7. What the research could not resolve

- Wronski's per-console split. The deck's text layer gives "around 1.1ms" without naming the console on that slide; the abstract names both PS4 and Xbox One.
- Whether AC4's "double resolution" means 128 slices or a larger XY grid. §4.2 reads it as 160×90×128, from the deck's own "160x90x64 or 160x90x128".
- The Vulkan-required minimum of `maxImageDimension3D`. The spec pages fetched in this pass were truncated before the required-limits table. The value 256 is recalled, not read **[U]**.
- Frostbite 2016 clouds numbers (PDF over the fetch limit) and RDR2's (232 MB PPTX).
- Timing for T2 and T3. None of the fetched sources publishes one. T1's was found in revision 2 (UE's Light Shafts page); T3 is derived from the listed texture rate (§4.4).
- How Frostbite voxelizes particles (raster or compute, scatter or gather). The transcript gives cost only.
- Whether AC4's "around 1.1ms" and UE's 1 ms / 3 ms include their apply pass (§4.2).
- A ray rate for the reference GPU, which would price per-froxel `rayQuery` visibility. None is in the tree or the fetched sources.
- UE's default Dynamic Shadow Distance and Volumetric Fog View Distance. The search budget was exhausted before they were fetched; the fetched pages give no defaults.
- The PS4's texture rate, which would add a third ratio to the cloud anchor (§4.1).

---

## 8. Sources

### By system

- **S1** — Wronski, *Volumetric Fog: Unified compute shader based solution to atmospheric scattering*, SIGGRAPH 2014 Advances. https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf (this pass: grid, range, ESM downsample, costs, extensions). Abstract: https://advances.realtimerendering.com/s2014/ . Survey article: https://www.gamedeveloper.com/programming/atmospheric-scattering-and-volumetric-fog-algorithm-part-1
- **S2** — Hillaire, *Physically Based and Unified Volumetric Rendering in Frostbite*, SIGGRAPH 2015. Transcript: https://www.slideshare.net/slideshow/physically-based-and-unified-volumetric-rendering-in-frostbite/51840934
- **S3** — UE volumetric fog: https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine
- **S4** — UE4 `BaseScalability.ini` copy: https://github.com/GameTechDev/UnrealCapabilityDetect/blob/master/Saved/Temp/Win64/Engine/Config/BaseScalability.ini
- **S5** — UE light shafts: https://dev.epicgames.com/documentation/en-us/unreal-engine/using-light-shafts-in-unreal-engine (revision 2: the GTX 680 cost, the occlusion-mode description, the off-screen and 90° behaviour)
- **S6** — UE local fog volumes: https://dev.epicgames.com/documentation/en-us/unreal-engine/local-fog-volumes-in-unreal-engine (revision 2: the `TileMaxInstanceCount` wording)
- **S7** — UE sky atmosphere: https://dev.epicgames.com/documentation/unreal-engine/sky-atmosphere-component-in-unreal-engine?lang=en-US (revision 2: directional-light shafts, *Cast Shadow on Atmosphere*, the shadow-distance recommendation)
- **S8** — UE volumetric cloud: https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-cloud-component-in-unreal-engine
- **S9** — UE heterogeneous volumes: https://dev.epicgames.com/documentation/en-us/unreal-engine/heterogeneous-volumes-in-unreal-engine
- **S10** — UE sparse volume textures: https://dev.epicgames.com/documentation/en-us/unreal-engine/sparse-volume-textures-in-unreal-engine
- **S11** — Coenen, *Doom Eternal — Graphics Study*: https://simoncoenen.com/blog/programming/graphics/DoomEternalStudy
- **S12** — Godot volumetric fog: https://docs.godotengine.org/en/stable/tutorials/3d/volumetric_fog.html
- **S13** — Unity HDRP fog override: https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@14.0/manual/Override-Fog.html
- **S14** — Bevy 0.14 notes: https://bevy.org/news/bevy-0-14/
- **S15** — Bevy `volumetric.rs`: https://github.com/bevyengine/bevy/blob/main/crates/bevy_light/src/volumetric.rs
- **S16** — Bevy fog module: https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/volumetric_fog/mod.rs
- **S17** — Schneider & Vos, HZD clouds: https://slideshare.net/guerrillagames/the-realtime-volumetric-cloudscapes-of-horizon-zero-dawn ; https://www.guerrilla-games.com/read/the-real-time-volumetric-cloudscapes-of-horizon-zero-dawn
- **S18** — Nubis, Evolved: https://www.guerrilla-games.com/read/nubis-evolved
- **S19** — Nubis³: https://www.guerrilla-games.com/read/nubis-cubed
- **S20** — Hillaire 2020 text mirror: https://www.readkong.com/page/a-scalable-and-production-ready-sky-and-atmosphere-3211109 (canonical https://sebh.github.io/publications/egsr2020.pdf, DOI 10.1111/cgf.14050; revision 2: the volumetric-shadow passage)
- **S21** — Hillaire reference code (MIT): https://github.com/sebh/UnrealEngineSkyAtmosphere
- **S22** — Bruneton 2017 re-implementation: https://ebruneton.github.io/precomputed_atmospheric_scattering/
- **S23** — Mitchell, GPU Gems 3 ch. 13: https://developer.nvidia.com/gpugems/gpugems3/part-ii-light-and-shadows/chapter-13-volumetric-light-scattering-post-process
- **S24** — NVIDIA Volumetric Lighting (legacy): https://developer.nvidia.com/volumetriclighting
- **S25** — Jendersie & d'Eon 2023: https://research.nvidia.com/labs/rtr/approximate-mie/
- **S26** — Engelhardt & Dachsbacher 2010 (metadata): https://api.crossref.org/works/10.1145/1730804.1730823
- **S27** — Chen et al. 2011 (metadata): https://api.crossref.org/works?query.bibliographic=Real-time+volumetric+shadows+using+1D+min-max+mipmaps
- **S28** — DiligentFX epipolar: https://github.com/DiligentGraphics/DiligentFX/tree/master/PostProcess/EpipolarLightScattering
- **S29** — Intel Outdoor Light Scattering: https://github.com/GameTechDev/OutdoorLightScattering
- **S30** — NanoVDB FAQ: https://www.openvdb.org/documentation/doxygen/NanoVDB_FAQ.html ; https://developer.nvidia.com/nanovdb
- **S31** — EmberGen: https://jangafx.com/software/embergen
- **S32** — Quilez, SDF soft shadows: https://iquilezles.org/articles/rmshadows/ ; height fog: https://iquilezles.org/articles/fog/
- **S33** — RDR2 abstract: https://advances.realtimerendering.com/s2019/index.htm ; Hillaire 2016 course page: https://blog.selfshadow.com/publications/s2016-shading-course/
- **S34** — Secondary implementations (grid presets, 0.95 history weight): https://www.mattiasstrand.com/posts/volumetric-fog/ ; https://www.euclideandreams.com/writing/building-a-froxel-volumetric-fog-renderer-in-unity-urp
- **S35** — GPU specs (this pass): https://en.wikipedia.org/wiki/GeForce_30_series (revision 2: the RTX 3060 Laptop texture rate) ; https://en.wikipedia.org/wiki/GeForce_900_series ; https://en.wikipedia.org/wiki/PlayStation_4_technical_specifications
- **S36** — GPU specs (revision 2): https://en.wikipedia.org/wiki/GeForce_600_series (GTX 680) ; https://en.wikipedia.org/wiki/GeForce_10_series (GTX 1080)

### In this tree (all re-opened at `6394bc5e`)

**`crates/boyko_render/src/`**

- `light.rs` — constants `:49`–`:85`, `:843`, `:848`; `ClusterConfig` `:856`; the dims pack `:956-959`; the kind word `:40-44`; `LightingConfig::exposure` `:562-564`.
- `render_path_config.rs` — `ShadowSources::HWRT_VIS` `:489-491`; `RenderPathConsumers` `:832`; `clusters_wanted` `:868`; `froxel_light_cull` `:1252`; `cap_forward_v1_consumers` `:1379`; the test `:3459`.
- `csm_config.rs` — `MAX_CASCADES` `:77`; `DEFAULT_SHADOW_DISTANCE` `:157`; `Shrink` `:218`; `CatchAll` `:222-223`.
- `motion_cam.rs:58`, `:101`, `taa_config.rs:58-62`, `taa_jitter.rs:53`, `ssao_config.rs:1-20`, `particle_plugin.rs:54-56`, `particle.rs:444`.

**`crates/boyko_rhi_vulkan/shaders/`**

- `cluster_cull.hlsl:101-103`, `:110-114`, `:376`, `:436-456`
- `light_table.hlsli:357-360`, `:376`
- `shadow_apply.hlsli:114-115`, `:151-153`, `:192`, `:226`, `:255-260`, `:281`, `:297`, `:313`, `:349`, `:396`
- `sdf_field.hlsli:48`, `:55`, `:84`, `:126-133`, `:151-162`, `:204-207`, `:210`, `:246`, `:287`
- `sdf_shadow_leaves.hlsli:49-67` (`:52`, `:56`, `:60`)
- `deferred_pbr.hlsl:508-511`, `:681`, `:954-955`, `:1178`, `:1236-1237`, `:1479`, `:1481`, `:1497`
- `forward_opaque.fs.hlsl:301-302`, `:436`
- `ddgi_resolve.hlsli:107`
- `pbr_lighting.hlsli:202`, `:237`
- `forward_sky.fs.hlsl:94`
- `vb_shade.comp.hlsl:520-521`
- `gbuffer_mrt.fs.hlsl:113`
- `viewt_from_depth.comp.hlsl:58`

**`crates/boyko_rhi_vulkan/src/`**

- `brick_atlas.rs:79-89`, `:131-132`
- `compute.rs:4330`, `:4340`
- `mesh_sdf_texture.rs:91-93`
- `ddgi.rs:86`, `:89`, `:92`, `:155-156`
- `device.rs:218`, `:392`, `:599`, `:669-672`, `:2814`, `:3925`
- `texture.rs:120-122`, `:263`
- `framegraph/graph.rs:353-360`
- `present/passes/gbuffer.rs:2360`
- `present/targets.rs:468-480`, `:2279`, `:2319-2323`
- `present/frame_driver.rs:48`
- `present/mod.rs:92`
- `present/passes/vb.rs:941-942`, `:1555`, `:3911-3919`
- `present/passes/forward.rs:62-63`
- `present/passes/particles.rs:514`
- `present/gpu_zone.rs`

**Other crates**

- `crates/boyko_rhi/src/enums.rs:457`
- `crates/boyko_rhi/src/device.rs:361`
- `crates/boyko_sdf_math/src/brick.rs:34`, `:38`, `:41`, `:195`
- `crates/boyko_sdf_math/src/lib.rs:132`, `:150-153`
- `crates/boyko_app/examples/room.rs:29`
- `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs:648`
- `crates/boyko_shaderdsl/src/shadow.rs:180`
- `crates/boyko_shaderdsl/src/emit/shaders.rs:903`
- `crates/boyko_shaderdsl/src/emit/mod.rs:546-552`
- `crates/boyko_ecs/src/ecs/core/app/plugin.rs:27`
- `crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:44`

**Tests**

- `tests/internal_docs_anchors.rs:349`

**Documents**

- [`OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md) `:149-157`, `:433`, `:451`, `:461`, `:463`, `:471`, `:489`, `:507`, `:525`, `:610`, `:853`
- [`SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md) `:84`
- [`diagnostics/W2208.md`](../diagnostics/W2208.md) `:29`
- [`POSTFX-AA-DESIGN-SPACE.md`](POSTFX-AA-DESIGN-SPACE.md) §5.2
- [`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md) `:640`, `:653-662`
- [`PARTICLES-PLAN.md`](../PARTICLES-PLAN.md) `:347`, `:486`, `:541`, `:547`, `:553`, `:2172`
- [`RENDER-AA-AND-TAILS-PLAN.md`](../RENDER-AA-AND-TAILS-PLAN.md) `:109`
- [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md) R4a, §2.2
- [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) R11, D8
- [`TRANSPARENCY-RESEARCH.md`](TRANSPARENCY-RESEARCH.md) `:91-92`
