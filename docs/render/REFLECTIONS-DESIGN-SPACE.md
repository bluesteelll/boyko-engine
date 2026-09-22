# Reflections — the design for THIS engine

> Status: architect's design, 2026-09-10, **pass 2 — after one critique pass and one audit of the
> pass-1 log itself** (§15 lists every finding and what pass 1 did with it: 5 blocking, 11
> non-blocking, **16 fixed, 0 refuted**; **§15.1 lists what pass 1's own log got wrong** — three
> rows claimed a repair to the companion RESEARCH that had not been made, and landing them
> uncovered a third instance of the same undercount. Every `file:line` the findings cite was
> re-opened at `ed0bed45`, and the censuses the critic ran — six ambient call sites, six
> `env_brdf_approx` definitions, eight tonemap producers — were re-run, not trusted).
> The survey it rests on is
> [`REFLECTIONS-RESEARCH.md`](REFLECTIONS-RESEARCH.md) — §0 there lists what the tree holds today,
> §2 numbers the techniques **T1–T42**, §5 numbers the pitfalls **P1–P45**; this document cites
> both by number. Every `file:line` below was re-opened at commit **`ed0bed45`** on
> `feat/multi-paradigm-render`. Numbers are either published elsewhere and cited with their rig,
> or **labelled estimates** — nothing was timed for this document and no `cargo` command was run.
> §13 separates the PERF/ARCHITECTURE decisions taken here from the VALUES/SCOPE decisions that go
> to the owner. The owner's instruction is "we will implement ALL of them", so §3 is a ladder with
> prerequisites, not a choice. The sibling campaign
> [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) shares one decision with this
> one (HDR scene colour, its R11 = this document's R4a) and is cross-referenced where it binds.
> This directory is not machine-anchored (`tests/internal_docs_anchors.rs:231`).

## 0. The shape in one paragraph

A reflection is **one radiance query per pixel, answered by a chain of sources that each return a
colour and a weight, composed until the weight reaches 1** — the rule every surveyed engine
converged on (RESEARCH T29: Filament's `Fr·(1−a) + E·ssr`, HDRP's weight-to-1, Godot 4.6's smooth
blend). In this engine the query already has ONE site: `eval_pbr_ambient_hemi`
(`crates/boyko_rhi_vulkan/shaders/pbr_lighting.hlsli:154-166`), whose `refl_color` is today a
two-colour gradient. The design replaces that gradient with the chain and never adds a second
lighting site. **Sources, bottom to top:** the distant environment (an octahedral HDR image with a
GGX-prefiltered mip chain + SH-9 irradiance + a 128² DFG LUT — rung R1, "metal looks like
metal"), local probes (an octahedral `D2Array` atlas with box/sphere parallax, clustered into the
froxel lists the light cull already builds — R2), screen-space rays over a **nearest-surface**
view-z pyramid built from the path-universal `gViewT` lane (R5–R7), SDF-traced rays in the field
the marcher already owns (R8), the DDGI irradiance grid for the roughness≈1 tail only (R9), and
hardware rays behind `feature = "hwrt"` (R10). **Data** is ECS-native throughout: environments and
LUTs are Gaia-baked byte blobs in append-only asset columns (the animation campaign's AK-6
request, shared), probes are components gathered into the light table as a new row kind, atlases
are the FFI/GPU-contiguity exception class the DDGI atlas already occupies (`ddgi.rs:1-12`), and
every config is an owner `Resource` + a derived `ResolvedX` carrier (`ssao_config.rs:1-14`).
**Shaders**: every new body is an eDSL leaf spliced into the existing files between sentinels;
a leaf whose body has no texture fetch carries an `f32` oracle, and a leaf that fetches inside
a loop is **emit-only** and gated by a device fixture — §4 says which is which per row, because
the eDSL's `ResRef` is an emit-only node (`emit/mod.rs:529-535`) and the claim "every leaf has
an oracle" would be the P43 class turned on this document. Nothing in the hand-written BRDF
core changes. **Nothing screen-space
ships before R4**, because two prerequisites are absent from the tree and not from the shading:
there is no HDR scene colour anywhere in the frame (RESEARCH §0.4) and the only depth pyramid has
the wrong polarity and lives on one path (§0.7). Those two are the campaign's real cost, and
they are paid once, early, and declared as a golden re-bless (P41).

## 1. Data model

### 1.1 The environment asset — `EnvAsset` (a baked blob, not a table)

One row per environment in an append-only, resource-owned column (the `ScratchColumn`-class
backing the animation design uses for `SkeletonBone`, `ANIMATION-DESIGN-SPACE.md` §1.1), indexed by
a bare `EnvHandle(u32)` carrier under the append-only / live-forever rule
(`crates/boyko_ecs/src/ecs/core/asset/handle.rs:41-44`):

```
EnvAssetHeader {                 // 32 B, #[repr(C)], POB — the hot row
    tile:        u16,            // octahedral map edge, power of two (256 default; §1.4)
    levels:      u8,             // prefiltered mip count INCLUDING level 0 (7 for 256: 256..4)
    format:      u8,             // 0 = B10G11R11_UFLOAT, 1 = R16G16B16A16_SFLOAT (§13.A D4)
    flags:       u32,            // bit 0 = sun baked into the map (§2.3 retires the disc)
    blob:        Range<u32>,     // byte range into the asset-blob region (AK-6)
    _pad:        [u32; 4],
}
EnvSh9 { c: [[f32; 4]; 9] }      // 144 B — a PARALLEL COLD column, same handle
```

The SH block is a separate column because it is read once per frame into the `EnvUbo` (§1.6)
and never per pixel, while the header is what the upload and the residency check touch —
hot/cold split by access pattern, the `SkeletonBoneName` cold-column move of
`ANIMATION-DESIGN-SPACE.md` §1.1.

The blob is the prefiltered octahedral chain, every mip complete **including its border texels**
(§1.4), laid out level-major, row-major, no padding. It is produced at **bake time on the host** by
the `f32` instantiation of the same eDSL leaves the GPU prefilter (R3) uses (§4) — so a baked
environment is bit-reproducible from its source and the bake is its own oracle. Source formats
the baker accepts: Radiance RGBE `.hdr` (in-house decoder, §6.2) and 16-bit PNG (already decoded,
`crates/boyko_image/src/lib.rs:9-14`) for LDR skies.

### 1.2 The DFG LUT — a committed `.bin`, the SMAA shape

`assets/ibl/DfgLut.bin`: 128×128, `Format::R16G16Sfloat` (`enums.rs` — the format exists), `.x =
scale`, `.y = bias`, indexed `(NoV, perceptualRoughness)`; **the same parameterisation the
analytic fit uses today** (`deferred_pbr.hlsl:582-589` takes `(roughness, NoV)`), so the swap is a
fetch for a polynomial and nothing upstream re-maps (P2). Uploaded at boot through
`upload_texture_2d_raw` (`crates/boyko_render/src/texture.rs:373-384`) after that function gains
a fourth `bpp` arm (`R16G16Sfloat => 4`), exactly as the SMAA `AreaTex`/`SearchTex` go up
(`smaa_luts.rs:44-50`). Generated by the `dfg_integrate` leaf's `f32` instantiation
(§4, 1024 Hammersley samples per texel — Filament's default, RESEARCH T2) and pinned by a
`dfg_lut_sync` test that regenerates and byte-compares. Energy compensation keeps its shipped
**form** and changes its **input**: `Ess = lut.x + lut.y`, `energy_comp = 1 + f0·(1/Ess − 1)`
(`deferred_pbr.hlsl:838-843`) — the Fdez-Agüera scale+bias estimate on a real integral instead of
a fit (P8). The channel convention is this engine's own; §14 records that Filament's
multiscatter-LUT channel meaning was not confirmed and is deliberately not depended on.

### 1.3 Probes — components, a light-table row kind, a parallel proxy table

```
#[component]                                  // table component; one per probe entity
ReflectionProbe {
    shape:      ProbeShape,                   // Box | Sphere — the parallax proxy (T18)
    half_ext:   [f32; 3],                     // box half-extents in probe space (sphere: .x = radius)
    influence:  f32,                          // influence radius; NDF blend starts at `influence·(1−blend)`
    blend:      f32,                          // 0..1, Lagarde's normalised-distance-field width (T19)
    priority:   i8,                           // manual override; ties broken by SMALLER volume first
    source:     ProbeSource,                  // Baked(EnvHandle) | Captured { every: u16 frames }
    intensity:  f32,
}
// + Transform (position + rotation = probe space)
```

At gather, every visible probe becomes **one light-table row of a new kind**, `LIGHT_KIND_PROBE
= 4` beside `LIGHT_KIND_SKY = 3` (`crates/boyko_render/src/light.rs:33-39`): the row carries
position, the influence radius as the cull sphere, and the index of a row in a parallel
`ProbeGpu` SSBO (64 B: inverse probe transform 3×4, half-extents, blend params, atlas layer,
flags). Why a light row and not a probe list of its own: the froxel cull already bins every
sphere-bounded light into per-cluster lists (`cluster_cull`, `MULTI-PARADIGM-RENDER-PLAN.md:489`),
`write_light_table` / `fold_light_table` / `upload_light_table` already exist
(`light_system.rs:181`, `:225`; `upload.rs:826`), `LightTableDirty` already gates the re-upload
(`light.rs:267`), and the loop in the resolve already dispatches on `kind` — so per-pixel probe
selection is the cluster walk the pixel does anyway, bounded by `MAX_LIGHTS_PER_CLUSTER = 256`
(`light.rs:72`) above and by a hard **4 blended per pixel** below (Godot Forward+, RESEARCH T19),
with **no linear search over all probes** (P33). The cull's bin test is
`sq_dist_point_aabb(CL.pos, cell) <= CL.range²` over a `LightElem`
(`cluster_cull.hlsl:322-326`), so a probe row with `pos = probe position, range = influence` is
binned by the existing function; what changes is the kind predicate, which is spelled `ck !=
LIGHT_KIND_POINT && ck != LIGHT_KIND_SPOT` at **three** sites (`:324`, `:357`, `:476`) and must
admit `LIGHT_KIND_PROBE` at all three — one omitted site is a probe that is gathered, uploaded
and never seen, the P39 class. Rows are written **size-ascending** (smaller
volume first) so the pixel's accumulate-to-1 loop terminates on the highest-priority probes
(Godot PR #100241, T19). **Where the order is made** (critique NB5): the per-froxel list
preserves light-table order — the cull scans the mask bits ascending into `uint local[256]`
and claims a contiguous slice (`cluster_cull.hlsl:342-374`), so the GPU never sorts and the order
is entirely the Rust writer's. The writer is `fold_light_table` / `fold_light_table_slotted`
(`light_system.rs:225`, `:273`), which folds the ECS iterators straight into the caller's `dst`
byte scratch with no intermediate `Vec` (`:179-181`) and writes rows kind by kind (`:290-305`
is the directional then sky loops). Probe rows are written LAST in the L0a span, and the sort is
an **in-place insertion sort over that probe span of `dst`** keyed by `half_ext` volume (with
`priority` as the primary key) — ≤ 64 rows of `GPU_LIGHT_BYTES`, swapped in the bytes already
written, **no side array, no allocation**, cold (once per `LightTableDirty` re-fold). Insertion
sort is chosen because the span is tiny and nearly sorted frame to frame; a Resource-owned key
array was considered and rejected as a second store for data the table already holds. RK-6 is
sized with this sort (§11). The 0%-gate is structural — a world with no `ReflectionProbe` writes
no row, the sort runs over an empty span, and every light-table golden is byte-identical.

### 1.4 The atlases — octahedral, bordered, hardware mips; two images

**Storage class**: the FFI/GPU-contiguity exception (`ddgi.rs:1-5`'s declared class), owned by a
`ReflectionTargets` struct in `present/targets.rs` beside `HzbTargets`.

| Image | Shape | Format | Content |
|---|---|---|---|
| `env_map` | 2D, `tile²`, `levels` mips | B10G11R11 (default) | the distant environment's prefiltered chain, uploaded from the `EnvAsset` blob **level by level** (RK-11, §3 R1 — no upload path in the tree today can fill a host-baked mip chain) |
| `probe_atlas` | `D2Array`, `128²` × `N` layers, 6 mips (128..4) | B10G11R11 | layer = probe; mip k = GGX at `env_lod⁻¹(k)` |
| `dfg_lut` | 2D 128² | RG16F | §1.2 |

**Octahedral, not cube** (§13.A D1). The map is a square whose texels decode through the eDSL's
own `oct_decode` (`crates/boyko_shaderdsl/src/oct.rs`; `emit/shaders.rs:2214`), the DDGI tile
discipline generalised to a mip chain: a `2^k` tile holds a `2^k − 2` interior and a **1-texel
border on every mip**, and the border is **written by the convolution itself** — the prefilter
evaluates each border texel at the direction its mirrored interior neighbour decodes to, so a
bilinear fetch at the seam reads the correct hemisphere on both sides. The chain stops at the
4×4 tile (2×2 interior), which is why 256 → 7 levels and 128 → 6; **no mip of any reflection
image is ever generated by a blit or an averaging pass** (P5 — auto-mips pull gutters into the
interior). A trilinear fetch between two bordered mips is then seam-safe on both levels. The
interior/border address arithmetic is the leaf `oct_env_uv` (§4), the `ddgi_irr_uv` rule with
the tile edge as a parameter.

**Layer count.** `MAX_TEXTURE_LAYERS = 16` (`crates/boyko_rhi_vulkan/src/texture.rs:117`) sizes an
inline **per-layer render-view** set for depth attachments and is asserted on every
`array_layers`. A probe atlas needs per-**mip** `D2Array` storage views (one view per level
spanning all layers — the `add_image_mipped` shape, `framegraph/graph.rs:392`) and no per-layer
view at all, so the cap is a kernel request (RK-3): build the per-layer view set only when the
usage carries an attachment bit, and let a sampled/storage array image go to 64 layers. Until it
lands the atlas holds 16 probes; the design does not work around it.

**Sizes (arithmetic, not measurements):** `env_map` 256², 4 B/texel, chain ≈ 4/3 → **350 KB**;
`probe_atlas` 64 × 128² × 4 B × 4/3 → **5.6 MB**; `dfg_lut` 64 KB. Against `taa_hist`'s 2 × 16.6
MB these are noise.

### 1.5 The screen-space images (appended LAST, `FRAMEGRAPH_IMAGE_COUNT` discipline)

Every image below is appended after the last `hwrt`-gated ResId so that ResIds 0..21 (hwrt) /
0..15 (not hwrt) stay byte-unchanged (`graph_bridge.rs:760-771`, the TAA/SSAO-append precedent);
each rung that adds one bumps `FRAMEGRAPH_IMAGE_COUNT` on BOTH legs and re-pins
`framegraph_gbuffer_equiv`.

| Image | Rung | Shape | Format | Notes |
|---|---|---|---|---|
| `lit` (re-typed) | R4a | full | **B10G11R11** when `lit_hdr_format_ok`, else **R16G16B16A16_SFLOAT** (was R8G8B8A8) | HDR scene colour; §8. `lit` is a STORAGE image on every compute producer (`targets.rs:123-127`; six `StorageImage` bind entries, `:4066`…`:5942`) and a COLOR ATTACHMENT on Forward (`:782-784`), and B10G11R11 storage is device-OPTIONAL (`device.rs:256-264`) — so the format is a boot probe with a stated fallback, RK-14 |
| `display` | R4a | full | R8G8B8A8 | the tonemapped + OETF output FXAA/SMAA/present read instead of `lit` |
| `refl_hiz` | R4b | `prev_pow2`, `MAX_HZB_LEVELS` mips | R32_SFLOAT | nearest-surface view-z pyramid (§2.2); `add_image_mipped` with an explicit seed |
| `refl_hit` | R5 | half | RGBA16F | `xyz` = hit position (world), `w` = hitT (≤ 0 = miss); the ratio-estimator input (T24) and the denoiser's hit distance (T40) |
| `refl_color` | R5 | half (R5) / full (R6 resolve) | RGBA16F | `rgb` = reflected radiance, `a` = confidence in [0,1] |
| `lit_prev` | R5 | full, ring ×2 | same as `lit` | previous-frame HDR colour, independent of TAA arming (P17); copied BEFORE `taa_resolve`, so it holds the **raster-jittered** frame and the trace subtracts the previous frame's jitter at the fetch (§2.4, D19) |
| `refl_hist` | R7 | full, ring ×2 | RGBA16F | the reflection denoiser's OWN history (P27), `a` = accumulated frames |
| `refl_var` | R7 | half, ring ×2 | R16F | luminance variance for the classifier and the temporal clamp |
| `refl_tiles` | R6 | buffer | u32 per 8×8 tile | tile classification + indirect args (T24) |

Memory at 1080p (arithmetic): `refl_hiz` 11 MB, `refl_hit` 4 MB, `refl_color` 8 MB, `lit_prev`
2 × 8.3 MB, `refl_hist` 2 × 16.6 MB, `refl_var` 2 × 1 MB → **≈ 74 MB at R7**. SSSR keeps ~13
targets (RESEARCH §4); this is nine.

### 1.6 Config, resolved carriers, UBOs

```
#[derive(Resource)] ReflectionConfig {            // the owner knob — the SsaoConfig pattern (ssao_config.rs:1-14)
    env:        EnvMode,      // Analytic (default, the 0%-gate) | Image
    probes:     bool,
    ssr:        SsrMode,      // Off (default) | Mirror | Stochastic
    denoise:    bool,
    traced:     TracedMode,   // Off | Sdf | Hardware (hwrt only)
    max_trace_roughness: f32, // 0.4 default (Lumen, T34); above it: probes/env only (P25)
    ssr_thickness: f32, ssr_max_steps: u32, ssr_half_res: bool,
}
#[derive(Resource)] ResolvedReflections { … }     // written by `resolve_reflection_policy`, cold, once at boot
                                                  //   (the path/legs are boot-frozen, FEATURE_MAP.md:122 — so is this)
#[component] EnvironmentLight { asset: EnvHandle, intensity: f32, yaw: f32 }   // at most one active; EMITS the SKY row (below)
EnvUbo (device, 208 B, ring-free — world-fixed like ResolvedDdgi's grid, ddgi_config.rs:13-19):
    sh9[9]: float4, params: float4 (intensity, yaw sin/cos, levels−1), flags: uint4 (sun_baked, mode bits)
```

The runtime mode gate is **light-header word 7, bit 7** (`light.rs:387-412` lists it as free):
`env_mode` (0 = analytic gradient, 1 = image chain). Everything else rides the `EnvUbo` and the
`ResolvedReflections` push data. `LIGHT_HEADER_WORDS` does not grow (it would re-encode every
golden, `light.rs:392-393`).

**The ambient call is reached only through a SKY row** (critique NB3). At all six sites the
ambient term is evaluated INSIDE the light loop's `LIGHT_KIND_SKY` branch (`deferred_pbr.hlsl:
1151-1163`; `pbr_lighting.hlsli:152-153` says the function is "TOKEN-FOR-TOKEN identical to the
`LIGHT_KIND_SKY` block"), and SKY rows come only from the `skies` iterator of the fold
(`light_system.rs:184`, `:228`, `:297-305`). A scene with an `EnvironmentLight` and no `SkyLight`
would therefore never evaluate the environment. The rule, D20: **the `EnvironmentLight` gather
writes the SKY row itself** — `LIGHT_KIND_SKY`, with `color`/`pos` (the sky/ground lanes) filled
from the SH-9 DC term split by hemisphere, so the row is complete for the `env_mode == 0` reader
too — and sets bit 7. Exactly ONE SKY row is written per fold: an `EnvironmentLight` takes
precedence and a co-present `SkyLight` writes no row (a `debug_assert!`-reported conflict in the
gather, reported once through the L8a-style counter, not per frame). Under `env_mode == 1` the
row's colour lanes are dead for the ambient (the chain replaces the gradient) and live for
`forward_sky`'s "no `SkyLight` entry" branch (`forward_sky.fs.hlsl:19`), which keeps that shader's
structure. Gate: a fixture with an `EnvironmentLight` and no `SkyLight` produces one SKY row with
bit 7 set; with both, still one row. The alternative — moving the ambient call out of the loop —
was rejected because it re-orders the `ambient` accumulation at six sites and moves every
pre-R1 pin for a scene that has no environment at all, breaking G5.

## 2. The seam — where reflections enter the frame

### 2.1 One lighting site, SIX splice points

The shipped ambient function is `eval_pbr_ambient_hemi` in the shared header
(`pbr_lighting.hlsli:154-166`). It has **six** consumers, one per `lit` producer, each hoisting
`R = reflect(−v, n)` once per pixel and each carrying its own verbatim `env_brdf_approx`
(critique B1 — pass 0 counted three consumers and two duplicates, which would have left two
paths on the gradient under `env_mode == 1` while the others showed the chain, the P39 class):

| # | File | `eval_pbr_ambient_hemi` call | `env_brdf_approx` def | Path |
|---|---|---|---|---|
| 1 | `deferred_pbr.hlsl` | `:1163` | `:582` | Deferred resolve |
| 2 | `forward_opaque.fs.hlsl` | `:303` | `:182` | Forward / ForwardPlus mesh leg |
| 3 | `sdf_forward_march.comp.hlsl` | `:1080` | `:855` | the SDF leg under Forward and VB (`:37-38`: "a TOKEN-FOR-TOKEN clone of `forward_opaque.fs.hlsl`'s own loop") |
| 4 | `vb_resolve.comp.hlsl` | `:368` | `:212` | VB **fused** (every VB boot with no pre-light consumer, `:1-8`; `:23` is the same token-for-token clone) |
| 5 | `vb_shade.comp.hlsl` | `:522` | `:247` | VB fused, material-classified (`vb_resolve` plus a pixel-selection prologue, `:11-14`) |
| 6 | `vb_shade_split.comp.hlsl` | `:540` | `:308` | VB **split** (pre-light consumers armed) |

The design adds `eval_pbr_ambient_env` beside the shared function — an eDSL-generated span, not
hand HLSL — with the same signature plus the chain's inputs, and the call site selects by the
word-7 bit:

```
float3 eval_pbr_ambient_env(Surface s, float3 R, float2 dfg, float spec_ao, float ao_final,
                            float3 sh_diffuse,            // SH-9 evaluated at n (leaf sh9_eval)
                            float3 env_spec,              // the chain's answer (§2.3), weight already 1
                            float  spec_weight)           // 0 when the chain returned nothing (never, by §2.3)
{
    float3 spec_ambient = (s.f0 * dfg.x + dfg.y) * env_spec * s.energy_comp;   // unchanged form
    float3 diff_ambient = s.diffuse_color * sh_diffuse;
    return diff_ambient * ao_final + spec_ambient * spec_ao;                   // the AO split is kept (P6)
}
```

The BRDF core (`D_GGX`, `V_SmithGGXCorrelated`, `F_Schlick`, `Surface`, `pbr_lighting.hlsli:61-116`)
is untouched. `env_brdf_approx` is not deleted: under `env_mode == 0` it still feeds `dfg`, which is
what keeps the analytic path byte-identical (G5). All six sites receive the SAME generated span
(the helper is "Duplicated (not shared)" by that codebase rule — `vb_resolve.comp.hlsl:26-30`
explains that a compute pass cannot share a raster FS's local helpers except textually), and
**one sync test pins all six**: it is a census, not a list — the test greps
`crates/boyko_rhi_vulkan/shaders/` for `eval_pbr_ambient_hemi(` calls and fails if the count of
call sites differs from the count of `eval_pbr_ambient_env` sentinel spans, so a seventh `lit`
producer added later cannot be missed silently the way the fourth and fifth were here. The
pass-0 sentence "one sync test pins all three" would have been green with two paths wrong.

### 2.2 The nearest-surface pyramid is built from `gViewT`, on every path

The occlusion HZB is `min` over hardware reverse-Z = the **farthest** surface, VB-only, early-pass
only (`hzb.rs:55-72`; `scene_types.rs:4246-4253`; P16). The reflection pyramid is a **different
image with a different source**: `gViewT` (`R32_SFLOAT`, "the marcher-aligned surface ray param
`t`", `targets.rs:128-133`) exists on every path and is what SSAO, SSCS and TAA already read. From
`t` and the per-pixel unit ray `rd` (`generate_ray`, `ray_gen.hlsli`), **view-z is one multiply**,
`z = t · dot(rd, cam_forward)`, and `min(z)` over a footprint is the NEAREST surface — the textbook
Hi-Z measure (T23), so the traversal needs no polarity argument at all.

**"Exists on every path" was wrong for the Forward family** (critique B4). The `viewt` image is
allocated on every path (`targets.rs:128-133`), but a PRODUCER exists only on two: Deferred×Mesh
(`viewt_from_depth`, armed iff `mesh_leg && !sdf_leg`, `graph_bridge.rs:1319-1330`) and VB×Mesh
(`vb_viewt` = `viewt_from_depth_rz.comp.hlsl`, `:3608-3609`, TAA-armed). `declare_forward_graph`
(`:2585`) declares NO pass that writes `viewt` and asserts it (`:2936-2941`: "the Forward graph
declares no viewt write for the marcher"); its depth prepass writes hardware reverse-Z
(`forward_opaque.vs.hlsl:16-17`), not `t`. So the pyramid builder is one body for four paths only
once Forward gains a producer, which is **RK-12**: (a) a `fwd_viewt` pass = the `viewt_from_depth_rz`
body over Forward's `depth` after the prepass — that shader already decodes exactly Forward's
encode (`viewt_from_depth_rz.comp.hlsl:5-12`: "`forward_view_proj_rows`'s encode … `view_z = B /
(d − A)`", and its header notes the decode is the one `sdf_forward_march`'s HAS_MESH variant
performs), so it is a declaration plus a dispatch, not a new shader; (b) on Forward×SDF legs the
VIEWT-variant marcher, whose `viewt` access the Forward declarator must declare first — the
tripwire at `:2936-2937` names this as the required step for any Forward-side `viewt` consumer.
Both are armed by the pre-light union (`ssr` is already a member, `render_path_config.rs:74-84`).
The participation matrix (§7) now says this per path instead of "prepass armed by the pre-light
union", which named the wrong producer.

The build pass reuses the
HZB's body (`hzb_build.comp.hlsl`; `prev_pow2` base, LDS reduce, `hzb_min` with the
`isnan(a) || isnan(b)` spelling because HLSL `min()` takes the OTHER operand on NaN,
`:122-130`, P30) with the source swapped and the identity swapped: an unwritten pixel (`mask == 0`,
sky) reads `+INF` so it never becomes the nearest surface. Declared with `add_image_mipped` and an
**explicit** `ResSync` seed (`framegraph/graph.rs:370-381` — the seed is required for the cross-frame WAR
reason stated there); built after the last `gViewT` producer of the path and before `resolve` /
`vb_shade` / the forward lighting pass. A host oracle (`hzb.rs`'s `conservative_min` shape) pins
"nearest" (G6): the same fixture under a `max` reduce is red.

### 2.3 The composition rule — weights to 1, energy-matched, one function

Per pixel, in the order below, each source returns `(rgb, w)`; the accumulate stops when
`Σw ≥ 1`; the last source is the environment with `w = 1 − Σw` (HDRP's "sky has weight 1"). This
is the leaf `refl_compose` (§4) and it is the ONLY place the sources meet:

1. **Screen-space** (R5+): `refl_color.rgb`, `w = refl_color.a` (edge/distance/backface/thickness
   fades folded in, Godot/Filament shape T26/T29). Roughness above `max_trace_roughness` has
   `w = 0` by construction (the classifier never traced it, P25).
2. **Traced** (R8/R10): the SSR-miss ray's answer, `w = 1 − w_ssr` for the roughness the trace
   covers, else 0.
3. **Local probes** (R2): up to 4 rows from the cluster list, weights by Lagarde's normalised
   distance field (centre = 1, boundary = 0, then renormalised — T19), parallax-corrected fetch
   at `env_lod(pR)`; `w_probe = min(1 − Σw, blend_sum)`.
4. **Rough tail** (R9): for `pR ≥ 0.7` (Godot's cutoff, T23) the DDGI irradiance at `n`,
   `w = 1 − Σw` — irradiance is the roughness→1 limit of the prefiltered chain and nothing
   glossier (P24).
5. **Environment** (R1): `env_map` at `env_lod(pR)` along the dominant direction (T11:
   `lerp(n, R, (1−pR)·(sqrt(1−pR)+pR))`, Unity's `GetSpecularDominantDir`), `w = 1 − Σw`.
6. **Analytic gradient** (today): only under `env_mode == 0`; then it is source 5.

All sources are sampled with the SAME `(f0·dfg.x + dfg.y)·energy_comp` weight applied ONCE, after
composition (§2.1) — that is what makes the roughness cutoff and the screen edge invisible (P32):
a source that returns nothing contributes exactly what the next source would have. The sun disc
(`deferred_pbr.hlsl:592-624`, `:1133-1150`, an intentional double-count with the direct lobe) is
retired to weight 0 by `EnvUbo.flags.sun_baked` when the environment contains the sun (P11), and
stays at `SUN_ENV_WEIGHT` otherwise; the analytic sky background (`:1409-1412`,
`forward_sky.fs.hlsl`, the `vb_sky` pass at `graph_bridge.rs:4585`) draws `env_map` mip 0 along
the view ray under `env_mode == 1`, so the background and the reflection are the same image.

**The diffuse side has a composition rule too** (critique NB2). Today DDGI is ADDED on top of the
hemisphere ambient: `deferred_pbr.hlsl:1178-1181` — "GI here is an ADDITIVE indirect term … the
L0a hemisphere/sky ambient already supplies the base" — and `ambient += diffuse_color * gi *
ao_final` (`:1195`) runs after the SKY block has already added `diff_ambient` (`:1163`). That is
tolerable while the base is a two-colour guess; once R1 makes it a real sky SH, a receiver inside
the DDGI grid counts the sky twice (the probes integrate the sky at their misses, the SH adds it
again) — the diffuse analogue of P11. The rule, D21: **the SH-9 term is weighted by the DDGI miss
weight.** `ddgi_probe_sample` already carries a coverage/convergence notion (its fallback argument
is the "no coverage" value, `:1189-1192`); R9 makes that weight explicit as `w_gi ∈ [0, 1]`
(1 inside a converged grid cell, 0 outside the AABB or where every corner is unconverged, the
trilinear weight sum in between) and the diffuse ambient becomes `diffuse_color · (gi + (1 −
w_gi) · sh_diffuse) · ao_final`. Under `ddgi_mode == 0` the term is `w_gi = 0` structurally and
the SH is the whole base (G5 holds); under `env_mode == 0` the hemisphere lerp takes the SH's
place in the same formula, so a DDGI-on analytic scene changes only by the `(1 − w_gi)` factor —
which is a declared change to the DDGI-armed goldens (there are no un-armed ones for it), not a
silent one. Gate: a furnace over a grid that fully covers the receiver ⇒ the diffuse ambient equals
the probe irradiance alone within `CHANNEL_TOL` (no sky double-count).

### 2.4 Frame order at the seam (every path; brackets = rung-gated)

```
… last gViewT producer (Deferred: marcher / viewt_from_depth; VB: marcher-VIEWT / vb_viewt;
                        Forward: fwd_viewt after the depth prepass + marcher-VIEWT — RK-12)
[R4b] refl_hiz_build            gViewT → view-z min pyramid                       (compute, per level)
[R6 ] refl_classify             thin-aux roughness + refl_var → refl_tiles + indirect args
[R5 ] refl_trace                Hi-Z march on refl_hiz, reads lit_prev[1-fi] → refl_hit, refl_color (half)
[R8 ] refl_trace_sdf            misses → field march (SDF legs) → refl_hit/refl_color rewrite
[R10] refl_trace_hw             misses → rayQuery closest-hit + hitT (hwrt)      (cs_6_5 variant)
[R6 ] refl_resolve              half → full ratio-estimator resolve (T24)
[R7 ] refl_reproject → refl_prefilter → refl_temporal   (the three-pass denoiser, T39; own history)
      resolve / vb_shade / vb_shade_split / forward lighting   ← eval_pbr_ambient_env (§2.1) reads refl_color
[R4a] lit_prev copy             lit[fi] → lit_prev[fi]   (after the last opaque producer; the transparency
                                campaign's `scene_color_copy` is the same copy — one pass, two consumers)
      translucent_draw (transparency campaign)                                     ← reads env/probes (R12)
      taa_resolve (pre-tonemap under R4a, TAA-PLAN.md:315 item 8)
[R4a] tonemap                   lit (HDR) → display (LDR, OETF)
      fxaa / smaa / present_sample   ← read `display`
```

SSR runs BEFORE the shading pass that consumes it and AFTER the geometry that produces thin-aux:
under VB that is after `vb_geo` and before `vb_shade_split` — it reads `lit_prev`, never the
current frame's `lit`, so the VB ordering constraint of P38 dissolves (there is no current-frame
colour to want).

**`lit_prev` is jittered, and the trace knows it** (critique NB4). The copy sits before
`taa_resolve` so that the reflection signal is independent of TAA (P17/P27) — which means the
copied frame was rasterised with that frame's `ndc_jitter` (`runner.rs:1544-1552` advances the
phase once per frame; `taa_jitter.rs:69-73` holds `phase`), while `MotionCam` is "ALWAYS built with
zero jitter" (`taa_resolve.comp.hlsl:21`). Reprojecting a jittered image through an unjittered
camera lands a mirror trace up to half a pixel off and shimmers with the Halton sequence. The
rule, D19: **subtract the previous frame's jitter at the fetch.** The R5 push data carries
`prev_jitter: float2` — the `NdcJitter` of the frame that produced `lit_prev[1 − fi]`, kept by
adding a `prev_phase`/`prev_armed` pair to `JitterState` (RK-13; `ndc_jitter` of the previous
phase, exact zero when the previous frame was not armed, the same structural-OFF rule) — and the
trace's screen-position of a reprojected point is `ndc_prev − prev_jitter` before the `lit_prev`
fetch. Two floats per frame; no history coupling. The alternative — copy post-TAA — was rejected:
it would make the reflection history a function of the TAA history (the P27 coupling both NRD and
SSSR reject) and would move `lit_prev` behind `taa_resolve` in the graph on TAA-armed boots only,
a second frame order to pin. G8 is extended: with TAA armed and the camera static, `refl_color` is
identical across the 8 jitter phases (a phase-invariance fixture; the un-corrected fetch fails it). The pre-light union already lists `ssr` (`render_path_config.rs:74-84`) and the
cap is lifted by `SsrMode != Off` feeding `ssr_on` (`:813-817`, `:1392`; the four pinned test
sites `:1843`, `:1944`, `:2061`, `:2643` are re-blessed in the same commit, P39).

## 3. The ladder

Each rung: what it **adds**, its **prerequisites in this tree** (`file:line`), its **gate** (red
first), its **cost** (labelled), and what it **closes**. Order is forced by prerequisites; R0 has
none inside the renderer.

### R0 — Foundations: the HDR decoder, the DFG LUT, the host bake

- **Adds**: `boyko_image::decode_hdr` (Radiance RGBE, §6.2); the eDSL leaves `dfg_integrate`,
  `ggx_prefilter_sample`, `env_lod`, `oct_env_uv`, `sh9_project` (§4) with their `f32`
  instantiations; the bake tool `boyko_ibl` (a `data`-profile Gaia bake step, §6.1) that turns an
  equirect `.hdr` into an `EnvAsset` blob + SH-9; `assets/ibl/DfgLut.bin` + `dfg_lut_sync`.
- **Prerequisites**: none in the renderer. The asset-blob region (AK-6, `format.rs:22-34` has no
  such region) is a prerequisite of *loading* the blob through Gaia, not of producing it — until
  AK-6 lands the blob loads through the same `include_bytes!` path SMAA uses (`smaa_luts.rs:44-50`),
  which is a stated interim, not a design.
- **Gate**: G1 furnace (§10) on the host: white environment, `f0 = 1`, roughness sweep 0..1, NoV
  sweep → `spec_ambient ∈ [1 − 2 %, 1 + 0.5 %]` with the LUT + compensation, and **red** with the
  analytic fit at `pR > 0.7`, `NoV < 0.2` (the P8 amplification, made visible). G3 bake/read
  identity. G2 LUT byte-sync.
- **Cost**: bake-time only. Host prefilter of a 256² chain: ~87 k texels × 64–1024 samples ≈
  **1–3 s per environment** (estimate, single thread, no SIMD; the leaf is lane-wise and can go
  8-wide later). Zero runtime.
- **Closes**: "the RHI is not ready" P42's third item; P7/P8.

### R1 — Split-sum IBL: "metal looks like metal"

- **Adds**: `env_map` + `dfg_lut` images and their explicit bindings on the resolve set (the
  DDGI @16/17/18 shape, `targets.rs:2527-2539` — the bindless table is `Texture2D`-only and bound
  only on the textured raster pipeline, `crates/boyko_rhi_vulkan/src/bindless.rs:1-3`, so the
  resolve gets explicit slots); the `EnvUbo`; a **trilinear** sampler — `SamplerDesc.mip` has
  only `MipMode::None` (`crates/boyko_rhi/src/device.rs:241-245`) and the one trilinear sampler in
  the tree is the bindless set's private `create_shared_sampler`
  (`crates/boyko_rhi_vulkan/src/bindless.rs:139`, `:158` `VK_SAMPLER_MIPMAP_MODE_LINEAR`), built
  below the RHI's `SamplerDesc`, so `MipMode::Linear` is RK-1 (two files share the basename
  `bindless.rs`; `crates/boyko_render/src/bindless.rs` is the slot allocator and says neither
  thing — critique NB8); `eval_pbr_ambient_env` at the six splice sites (§2.1); `sh9_eval`; the
  dominant-direction lerp (T11); the background draw from mip 0; the word-7 bit-7 gate; the
  `EnvironmentLight` gather that writes the SKY row (§1.6, D20).
- **Prerequisites**: R0; RK-1 (sampler mip mode) — without it the chain is sampled at level 0 and
  every roughness reads as a mirror, which is exactly the P1 class of silent wrong; **RK-11, a
  per-level raw upload** (critique B3) — the blob is a host-baked chain whose every mip, borders
  included, is data (D2 forbids a blit or an averaging pass), and no upload path in the tree can
  fill one: `upload_texture_2d_raw` creates a single-level image (`texture.rs:373-386`, `mip_levels:
  1` at `:403`) and `upload_texture_2d` builds mips by a LINEAR blit chain (`:164-168`). The
  primitive exists one layer down — `BufferImageCopy.mip_level` (`crates/boyko_rhi/src/
  descriptor.rs:628`) and the level-0 copy `upload_mipped_pixels` already issues (`texture.rs:
  246-262`) — so RK-11 is `upload_texture_2d_raw_mipped(ctx, w, h, levels, bytes, format)`: one
  staging buffer, `levels` buffer→image copies at `mip_level = k` with the level-major offsets of
  §1.1, one barrier pair, no blit. RK-2 (the bpp arm) is folded into it. Pass 0 named neither,
  which made R1 "a rung whose prerequisite is not in this tree and not named".
- **Gate**: G5 0%-gate (no `EnvironmentLight` ⇒ all 30 pins byte-identical,
  `FEATURE_MAP.md:1059`); G4 seam (a fetch 0.5 texel outside the interior equals the fetch at the
  mirrored direction within `CHANNEL_TOL`); a NEW image golden `grand_showcase_ibl` for the armed
  path; G3 on the device (a `pR` sweep on a chrome sphere reads mip `env_lod(pR)` — the Godot
  #69514 mismatch is a red fixture, P1).
- **Cost** (estimate): per pixel 1 trilinear fetch (4 B texels, 350 KB chain — L2-resident) + 1
  bilinear LUT fetch + ~30 ALU of SH; ≈ 2 M px × ~12 B ≈ 25 MB of mostly-cached reads at 1080p
  → **0.05–0.15 ms** on an RTX-3060-class part (bandwidth arithmetic at 100–200 GB/s effective).
- **Closes**: T1–T5, T8, T9 (real input), T11, P7–P9, P11; the memory
  `project-pbr-quality-diagnosis` failure ("raise the ambient until it looks right").

### R2 — Local probes with parallax, clustered, weight-composed

- **Adds**: `ReflectionProbe` component + gather into `LIGHT_KIND_PROBE` rows + the `ProbeGpu`
  SSBO (§1.3); `probe_atlas` (§1.4); the leaves `parallax_box`, `parallax_sphere`,
  `probe_blend_weight`, `refl_compose` (sources 3 and 5 live); the cluster-walk branch in the
  six shading bodies (§2.1) — on plain `Forward`, `vb_resolve`, `vb_shade`, `vb_shade_split` and
  `sdf_forward_march` it is a flat walk over `[l0a_count, light_count)`, see §7; **baked** probes
  only (a `Baked(EnvHandle)` source whose blob is captured
  offline by the bake tool through a 6-face software render — the marcher for SDF legs, the raster
  path for meshes — then resampled cube→oct on the host).
- **Prerequisites**: R1; RK-3 (layer cap) for more than 16 probes; the froxel cull accepting a
  fifth kind (`cluster_cull.hlsl` sphere bound — the point-light path). Runtime **capture** is
  NOT here: it needs the per-view seam (`MULTI-PARADIGM-RENDER-PLAN.md:497` R11 "reflection-probe-view
  forward vs main deferred smoke"), which is R3's prerequisite.
- **Gate**: a fixture with two overlapping probes and a pixel at each centre → 100 % from that
  probe (Lagarde's constraint); a pixel on a boundary → 0 %; the size-ascending order is a
  red-first test (two probes swapped in the table ⇒ different colour at the overlap); the
  cluster count of probe rows == gathered probes.
- **Cost** (estimate): +1 trilinear fetch per contributing probe (≤ 4) + ~20 ALU of box
  intersection each; **+0.05–0.2 ms** at 1080p when half the screen is inside probes. Lagarde's
  0.25 ms/cubemap at 25 % coverage is PS3-era [B] and is not a prediction.
- **Closes**: T17–T19, P33–P35 (P34 popping is mitigated by the blend width, not solved — Lagarde's
  open limit, §14).

### R3 — GPU prefilter: dynamic environments and captured probes

- **Adds**: the compute chain `env_copy → env_downsample (bordered) → env_ggx_convolve[k] →
  env_sh_project` over storage views of `env_map` / `probe_atlas` levels (the HZB per-level view
  set, `targets.rs:1436-1447`, is the template — "the engine's FIRST storage image with a mip
  chain"); filtered importance sampling `lod = 0.5·log2(Ωs/Ωp)` with a full bordered source chain
  first (T5, P3); a `Captured { every }` probe source rendered through the per-view seam into a
  transient 6-face `D2Array` then `cube_to_oct` resampled; time-slicing at the Unity budget shape
  (one face or one mip per frame, T20).
- **Prerequisites**: R2; the per-view seam (MPR R11); `DeviceCaps` probes for B10G11R11 storage
  (`ddgi_irr_storage_ok`, `device.rs:256-264` — the house rule P12; on `false` the atlas is
  RGBA16F, `R16G16B16A16Sfloat` storage is Vulkan-core mandatory per `enums.rs:362-364`).
- **Gate** (rewritten after critique B5 — as first written it could not pass): the GPU chain
  equals the host bake **bit-for-bit**, and that is a real target only because of HOW the chain
  reads its source. A hardware `SampleLevel` has no bit-exactness contract against a host
  bilinear, and the tree's one sampled-radiance precedent says so in as many words
  (`ddgi_probe_gi_sync.rs:123`: "Tolerance (not bit-exact): the marched-radiance write is
  GPU-golden + tolerance regardless"); a bit-exact gate over a sampler fetch is a gate that
  cannot pass, and "widen until green" is this repository's catalogued failure. So the
  convolution leaf `env_ggx_convolve` does **not** sample: the source level is bound as a
  `StructuredBuffer<uint>` of packed texels (the `sdf_field.hlsli:13-18` include contract, the
  eDSL's `BufferLoad` node, `emit/mod.rs:291`) and the leaf does its own **bilinear/trilinear in
  the body** — `oct_env_uv` → four `BufferLoad`s → unpack → lerp — under the `sdf_field.hlsli:
  21-31` determinism contract (plain IEEE ops, no FMA contraction, no `rsqrt`/`rcp`, no FP16).
  `BufferLoad` is an ordinary node with an `EvalCf` arm, so the WHOLE loop — sample directions,
  FIS `lod`, the fetch, the accumulate — is one body instantiated over `f32` and `Emit`, and the
  host bake IS that `f32` instantiation (D13 now holds for the loop, not only for the per-sample
  math). The image-shaped source (`env_map`'s storage views) is written by `env_copy`, which
  packs the buffer form back; the buffer mirror is transient per convolution and lives in the
  FFI/GPU-contiguity class. Cost of not sampling: four loads + a lerp per sample instead of one
  filtered fetch, on a 350 KB L2-resident source — accepted for a pass that is amortised (§13.B
  4). A later `SampleLevel` arm is permitted ONLY as a `-D` variant with a tolerance gate stated
  per level and a manifest row, never as the gated form. The firefly gate stays: a 1e4-cd sun
  texel in a 256² source produces no texel above the analytic bound at mip 3 (P3).
- **Cost** (estimate): Bevy's `32·2^(4r)` samples per texel per level, every frame (T6): for 256²
  that is ~87 k texels × ~100 samples average ≈ 9 M fetches ≈ **0.3–0.8 ms** when re-filtered
  every frame; amortised over 8 frames ≈ 0.1 ms with a one-frame-per-mip lag (P14 — the lag is
  stated, and the owner picks the cadence, §13.B).
- **Closes**: T6, T20; the dynamic sky.

### R4a — HDR scene colour and the tonemap at the tail (the coupled decision)

- **Adds**: `lit` re-typed to **B10G11R11** (§8; `Format::B10G11R11UfloatPack32` exists,
  `enums.rs:365-376`) behind the boot probe of RK-14; the `display` image; a `tonemap` pass (ACES
  fit + `pow(1/2.2)` moved out of **eight** producers into one fullscreen pass — critique NB6:
  `grep -l 'OETF_GAMMA_EXP|tonemap_select'` over `shaders/` is `deferred_pbr.hlsl`,
  `forward_opaque.fs.hlsl`, `forward_sky.fs.hlsl`, `sdf_forward_march.comp.hlsl`,
  `ssaa_downsample.fs.hlsl`, `vb_resolve.comp.hlsl`, `vb_shade.comp.hlsl`, `vb_shade_split.comp.hlsl`,
  plus the definition in `pbr_lighting.hlsli:181-209`; pass 0 said three). The SSAA leg is the
  one that changes shape rather than only losing a tail: today `ssaa_downsample` decodes, box-
  averages and re-encodes display-space texels (`:8-16`) and its header already records the
  ceiling "avg(tonemap(x)), not tonemap(avg(x))" (`:17-19`); under R4a it becomes a plain HDR
  box filter `lit(2×) → lit(native)` and the `tonemap` pass runs after it, so the ceiling is
  closed by the move and the decode/encode pair is deleted. TAA moves pre-tonemap with
  luma-weighted blending (`TAA-PLAN.md:315` item 8 — the coupling the plan itself booked);
  FXAA/SMAA/present read `display`; the `lit_prev` copy.
- **Prerequisites**: **RK-14** (critique B2). `lit` is a STORAGE image for every compute producer
  (`targets.rs:123-127`; created from `GBUFFER_FORMAT` with a caller-passed usage, `:7236-7252`,
  `ImageUsage::STORAGE` at `:7695`/`:8885`) and a colour attachment for Forward (`:782-784`), and
  `STORAGE_IMAGE` on B10G11R11 is a device-OPTIONAL feature the tree already probes and DEGRADES
  on (`device.rs:256-264`, `ddgi_irr_storage_ok`, "RECORDED, NOT a boot fail-fast"). Pass 0
  applied that probe to R3's atlas only and left R4a with no boot outcome on a device without the
  feature. RK-14 is `DeviceCaps::lit_hdr_format_ok`: B10G11R11 supports BOTH `STORAGE_IMAGE` and
  `COLOR_ATTACHMENT` under OPTIMAL tiling (the `viewt_storage_format_ok` probe shape, `device.rs:
  3266`, widened to two usage bits — the attachment bit is probed rather than assumed because §14
  records that the mandatory-format table was not served). On `false` the outcome is DECIDED:
  `lit`, `lit_prev` and the SSAA 2× ring are **`R16G16B16A16_SFLOAT`**, whose storage support is
  Vulkan-core mandatory (`enums.rs:362-364`) — a degrade to 2× bandwidth, never a fail-fast (the
  engine must boot on either path, the `atlas_linear_filter_ok` rule at `:250-254`). The format
  is read from `ResolvedReflections` at target creation, once; the shaders are unchanged (both
  formats are `float4` storage in HLSL, only the `[[vk::image_format]]` / view format differs,
  which is a `-D LIT_FORMAT` variant row). D4's "bandwidth parity" is therefore **conditional on
  the probe** and §8 says so. Otherwise none technical — it is a **scope event**: every one of the
  30 pins re-blesses by construction (P41), and `material.rs:56-60`'s "UNORM end to end, the OETF
  must be manual" becomes "float until the tail". `TRANSPARENCY-DESIGN-SPACE.md` R11 is the SAME
  decision (its ballot 1); the two campaigns land it once.
- **Gate**: `tonemap(lit_hdr)` byte-equals the old `lit` for a scene whose radiance is inside
  8-bit range with `exposure = 1` — a **transition pin**: the pre-move `.bmp` hash is the
  post-move `display` hash for the un-armed configuration, so the re-bless is checked, not
  trusted, and it is run once per **producer** — eight transition pins, one per file in the list
  above, because a producer that keeps its tail would pass a single deferred-path pin while
  double-tonemapping on its own path; a highlight fixture (a 50× sun) whose `lit` clips today and
  does not after; the RK-14 fallback arm is exercised by forcing `lit_hdr_format_ok = false` in a
  test boot (the `DeviceCaps` test-constructor shape, `device.rs:4107`), and the transition pin
  must hold on that arm too.
- **Cost**: B10G11R11 is 4 B/px — **bandwidth parity with RGBA8 when the probe is true**; on the
  RGBA16F arm every `lit` producer and consumer moves 2× the bytes (≈ +8 MB per full-screen
  pass at 1080p), stated rather than hidden; the tonemap pass is one fullscreen read+write (≈ 12
  MB, **≈ 0.05 ms**, estimate); `lit_prev` copy ≈ 8 MB (16 on the fallback arm).
- **Closes**: P15, P17 (the previous-frame colour is now independent of TAA), the transparency
  campaign's R5/R8 physics.

### R4b — The nearest-surface pyramid (§2.2)

- **Adds**: `refl_hiz` + `refl_hiz_build` on all four paths (Deferred, Forward, ForwardPlus, VB)
  with the identity swapped; `hzb_build.comp.hlsl`'s body parameterised by `-D HZB_SOURCE_VIEWT`
  — one authored body, one more manifest row (P40).
- **Prerequisites**: a `gViewT` PRODUCER on the path — present on Deferred×Mesh and VB×Mesh and
  on the SDF legs of Deferred/VB, ABSENT on Forward and ForwardPlus until **RK-12** lands (§2.2;
  `graph_bridge.rs:2936-2941` asserts the absence). Pass 0 wrote "present on every path", which
  was true of the image and false of its writer. `ResolvedReflections.ssr != Off` arms the
  builder and, through the pre-light union, the Forward producer (a consumer arms its producer —
  the W4 class, `render_path_config.rs:94-100`).
- **Gate**: G6 polarity oracle (host `min` over view-z from a fixture depth == the shader's
  level-k texel, and the `max` variant is red); the NaN census (`isnan` spelling present, no
  `OpFMin` in the `.spv`, the `hzb_build` census pattern).
- **Cost** (estimate): the HZB build's own shape over 8 MB → **≈ 0.1 ms** (SSSR's SPD is 0.12 ms
  on an RTX 3080 Mobile, RESEARCH §4).
- **Closes**: P16, P29 (the epsilon/origin rules are copied from the sugulee notes into the
  leaf's doc, [B] recorded).

### R5 — Mirror SSR on the pyramid

- **Adds**: `refl_trace` (Hi-Z traversal, T23: `mip_offset = hit ? −1 : +1`, thickness at mip 0
  only, cell-boundary epsilon, back-facing ray inversion), reading `lit_prev[1−fi]` reprojected
  through `MotionCam` (`motion_cam.rs:1-20`, camera-only — the static-geometry contract TAA v1
  already accepts, `taa_resolve.comp.hlsl:13-28`); `refl_hit` / `refl_color` at half res with a
  surface-aware bilateral upsample (Godot 4.6, T23); confidence = edge fade (5 % margin) ×
  distance fade × thickness × backface; `SsrMode::Mirror` lifts the cap; thin-aux `ROUGHNESS`
  produced on every path (the VB packs it in `thin_normal.B` — `vb_geo.comp.hlsl:44-53`; the
  `render_path_config.rs:446` doc says `.BA` and is corrected in the same commit, RESEARCH §0.6
  C6); the untextured Deferred path writes roughness into `gMaterial`'s spare lane or reads it
  from the material SSBO by id as the resolve already does (`deferred_pbr.hlsl:799-801`).
- **Prerequisites**: R4a (HDR colour — P15), R4b, R1 (the fallback must exist before the thing
  that falls back to it — RESEARCH P32, "shipping SSR before the fallback it degrades into").
- **Gate**: G7 seam energy (confidence 0 ⇒ `refl_compose` == the R1 result bit-for-bit); a mirror
  floor fixture with a known object → the reflected pixel column equals the object's `lit_prev`
  column at the analytic intersection; the four `ssr_on` assertions re-blessed (P39).
- **Cost** (estimate): SSSR's intersect is 0.88 ms at 1080p full-rate on an RTX 3080 Mobile;
  half-res mirror-only with a 0.4 roughness gate on this box **≈ 0.5–1.0 ms**. Godot's half-res
  Hi-Z is 3.10 ms on Sponza / RTX 3070 Ti at 64 steps — a bound, not a target.
- **Closes**: T22/T23/T26, P28/P29, the `ssr_on` cap.

### R6 — Stochastic SSR: VNDF rays, classification, ratio-estimator resolve

- **Adds**: `vndf_sample` (Heitz 2018 spherical-cap form, T5/T38 — NRD's "VNDF v3" requirement);
  a blue-noise source (a 128² Owen-scrambled Sobol tile baked at R0 as a second `.bin`; IGN
  `deferred_pbr.hlsl:636-644` is the interim and is NOT the shipped form — white/IGN converges
  visibly slower, T40); `refl_classify` over 8×8 tiles with `samplesPerQuad` and
  variance-guided escalation to 4 rays/quad (T24); `refl_resolve` — the full-res BRDF-weighted
  reuse of neighbouring half-res hits ("a ratio estimator", Stachowiak); `SsrMode::Stochastic`.
- **Prerequisites**: R5; an indirect-dispatch path for the tile buffer (the VB cull chain has one).
- **Gate**: a variance-reduction gate — the resolve's per-pixel variance on a glossy fixture is
  below the trace's by the neighbourhood factor (a count, not a clock); the classifier's tile
  count equals the roughness census of the fixture.
- **Cost** (estimate): classify 0.17 ms + trace 0.88 ms + resolve (SSSR does not publish it
  separately; the prefilter is 0.34 ms) on the 3080-Mobile anchor → **≈ 1.5 ms** here before the
  denoiser.
- **Closes**: T24, P25/P26.

### R7 — The reflection denoiser (its own history, dual reprojection, demodulation)

- **Adds**: three passes — `refl_reproject` (surface motion from `MotionCam`, **virtual motion**
  from `refl_hit.w` along the dominant direction, blended by roughness toward surface motion at
  `pR = 1`, T28/T38), `refl_prefilter` (16 Halton taps weighted by normal / view-z / radiance /
  variance, T39), `refl_temporal` (variance clamp + disocclusion by prev view-z, the 3b shape
  with a 3-channel HDR signal); **material demodulation** — the trace outputs radiance
  divided by `(f0·dfg.x + dfg.y)` and the compose multiplies it back (T40); `refl_hist`,
  `refl_var`; an anti-firefly clamp (HDRP's `Clamp Value`); the **unjittered** camera ring the TAA
  path already keeps (`targets.rs:494-498`, P21) bound to all three.
- **Prerequisites**: R6; per-object motion for moving reflectors is `hwrt`-gated today
  (`motion_cam.rs:38-45`) — the denoiser ships with camera-only motion and the same surfaced
  limitation TAA v1 carries; the per-object half un-walls with the animation campaign's prev-position
  ring (`ANIMATION-DESIGN-SPACE.md` §1.8), which is the first consumer that needs it on every build.
- **Gate**: G8 separation (TAA off vs on ⇒ `refl_color` identical — one accumulator per signal,
  P27); G9 demodulation identity (a constant-radiance environment ⇒ the denoised output is
  texture-free on a textured metal); a disocclusion fixture (an object revealed at frame N has
  `refl_hist.a == 1` at N).
- **Cost** (estimate): the SSSR denoiser is 1.53 ms of 2.70 at 1080p on the 3080 Mobile (57 %,
  P22) → **≈ 1.5–2 ms** here; memory §1.5.
- **Closes**: T37–T40, P18–P23, P27.

### R8 — SDF-traced reflections for SSR misses (analytic fold now, brick atlas when P9 lands)

- **Adds**: `sdf_reflect_march` — a leaf of the `sdf_soft_shadow_ranged` shape
  (`sdf_shadow_leaves.hlsli:49-51`) over `field_distance` (`sdf_field.hlsli:246`, with the
  `StructuredBuffer<uint> Buf` include contract at `:13-18`), returning hitT + normal; hit shading =
  the direct sun with the analytic soft shadow + the R1 environment (a "hit-lighting lite",
  T34's cache-less arm); misses fall to sources 3–5. On `TracedMode::Sdf` only SSR-miss pixels
  of the SDF leg trace; mesh geometry is invisible to the field (T31).
- **Prerequisites**: R5 (the miss mask); `MAX_SDF_EDITS = 16` (`sdf_field.hlsli:48`) bounds the
  fold at the audit's ~4,300 evals/pixel/frame figure (`SDF-PERF-AUDIT.md:82-85`). The brick
  atlas swap is `field_skip` (`:248-254`, "Source-only for W0 — no shader yet calls it") and its
  sampler is NEAREST today (`brick_atlas.rs:80-84`, BUG-M2-GPU-1) — a cone/glossy trace over the
  atlas needs its own filtering and is R11's problem, not this rung's.
- **Gate**: the SDF-only golden: a mirror plane in front of an SDF sphere reflects the sphere at
  the analytic intersection (a host oracle exists for the field); the edit-count cost curve is a
  bench IN the gate (`feedback-optimal-solution-and-bench-in-gate`), reported per rung.
- **Cost** (estimate): steps × 16 edits per ray for the miss fraction only; at a 20 % miss rate
  and 48 steps ≈ 0.4 M rays × 48 × 16 ≈ 300 M primitive evals → **≈ 0.5–1.5 ms**; the audit's
  n = 256 wall (~68,000 ops/sample) is why the atlas is the second half of this rung.
- **Closes**: T31/T32; Lumen's software arm.

### R9 — DDGI-fed rough reflections (the roughness≈1 tail, nothing glossier)

- **Adds**: source 4 of `refl_compose`: for `pR ≥ 0.7` and no SSR/trace answer, the DDGI
  irradiance at `n` through `ddgi_probe_sample` (`ddgi_resolve.hlsli:107`, bound @16/17/18) —
  which is what the prefiltered chain converges to at roughness 1; a smooth ramp 0.6..0.7
  between the chain and the grid so the cutoff is not a ring (P32).
- **Prerequisites**: R1; DDGI armed (`ddgi_config.rs:29-33`, default off — the source has
  `w = 0` when the grid is off, structurally).
- **Gate**: a furnace over the grid (constant-radiance world ⇒ the tail equals the chain's top
  mip within `CHANNEL_TOL`).
- **Cost**: one probe sample the pixel may already pay for diffuse — **≈ 0**.
- **Closes**: T35 in its cheapest form; P24 is the boundary of this rung, and a **directional
  radiance atlas** (the SDFDDGI plan's "atlas-format revisit", `RENDER-SDFDDGI-PLAN.md:23-24`)
  plus a cone trace over it (T33) is R11.

### R10 — Hardware-ray reflections behind `feature = "hwrt"` (owner-eval only)

- **Adds**: `refl_trace_hw` — the existing inline `RayQuery` promoted from
  `ACCEPT_FIRST_HIT_AND_END_SEARCH` visibility (`deferred_pbr.hlsl:1038-1042`) to closest-hit with
  `hitT` and the instance/primitive ids; `RayWorkload::Reflection` (`ray_backend.rs:67-76`) routed
  to `HardwareTriBvh` per `RENDER-HYBRID-RAY-SYSTEM-DESIGN.md:205` with SSR as the
  `RtTier::Absent` fallback; a `cs_6_5` variant row.
- **R10b — hit shading is a sub-rung with its own cost, not a line item** (critique NB9). Pass 0
  wrote "hit shading through the material table by instance = Lumen's hit lighting — no surface
  cache" and stopped. Epic's page [D] (opened for this revision:
  `dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine`)
  describes the Surface Cache as the structure "used to quickly look up lighting at ray hit
  points", used by default because it "is significantly faster to render", and Hit Lighting as
  the alternative that evaluates lighting at the hit "at a performance cost" — so choosing the
  cache-less arm is choosing the expensive one, and the rung must say what a hit costs here. At a
  hit `(P_hit, n_hit, material)` the shading is: **direct** = the primary directional × its
  visibility, where visibility at an arbitrary world point is the CSM lookup the shading bodies
  already perform for the pixel's own `P` (a function of world position, so it works at `P_hit`
  inside the cascade range) on mesh legs, the analytic `sdf_soft_shadow` on SDF legs, and
  **unshadowed beyond the last cascade** (stated: a second `rayQuery` toward the sun is the
  correct answer and is a `-D HIT_SHADOW=2` variant, doubling the ray count); **ambient** = the
  R1 environment at `n_hit` (`eval_pbr_ambient_env` with `w_ssr = 0` — no recursion) plus the DDGI
  irradiance at `P_hit` when the grid is armed (R9's `ddgi_probe_sample` takes `(P, n)` and needs
  no screen-space input); **no** point/spot lights at the hit in v1 (the froxel list is indexed
  by the PIXEL's cluster, not the hit's — a world-space cluster lookup at `P_hit` is the R10c
  door). Variant rows: `refl_trace_hw` × `HIT_SHADOW ∈ {0 unshadowed, 1 csm/atlas, 2 ray}`, each
  in `SHADER-VARIANT-MANIFEST.md`. This is the "hit-lighting lite" R8 already uses for SDF hits,
  now written down once and shared by both traced arms.
- **Prerequisites**: R6/R7 (the denoiser is what makes 1 ray/pixel usable); R1 and R9 for the
  hit's ambient; the TLAS per FIF (`boyko_app/src/gpu_scene/tlas.rs:8-22`); `feature = "hwrt"`.
- **Gate**: owner-eval only — `RENDER-HWRT-OPTIONAL-ANALYSIS.md:155-162` makes the HW path
  untestable in CI on this box; the software arm's golden is the gate for everything but the
  traversal body (the §6 rule of the hybrid design: one authored body, owner-eval per body).
- **Cost**: unmeasured and unpublished for this GPU class; §14.
- **Closes**: T34; the last arm of the fallback chain.

### R11 — Glossy traced reflections: radiance atlas + cone trace (research rung)

- **Adds**: a directional radiance probe atlas (oct tiles storing radiance, not irradiance — the
  DDGI atlas-format revisit), and an SDF/atlas **cone** trace with aperture from roughness over
  its mips (T33, noiseless — no denoiser); alternatively Radiance Cascades (T42) if convergence
  limits, as the SDFDDGI plan already books.
- **Prerequisites**: R8, R9; the brick atlas with a LINEAR sampler (its own campaign, P9 of the
  SDF audit).
- **Gate / cost**: to be designed at its own pass; recorded here so the ladder is complete.

### R12 — Transparents, planar, material extensions

- **Reflections on transparents**: the translucent FS of the transparency campaign calls the
  SAME `eval_pbr_ambient_env` (its §4 pass uses the opaque BRDF's ambient term by construction),
  so R1/R2/R9 reach glass **the day they land** with zero transparency-side work; SSR on glass is
  front-layer only, matched by the depth threshold the transparency design's R3 identifies
  (`TRANSPARENCY-DESIGN-SPACE.md` R10; HDRP's "reflect the depth behind" failure is P-class and
  stated). Transparents are never in `lit_prev`'s trace target (they draw after the copy, §2.4)
  — the HDRP `BeforeRefraction` rule, chosen rather than inherited.
- **Planar**: a mirrored second view through the per-view seam with Lengyel's oblique near plane
  (T21); "budget half your frame" (UE) — an owner ballot, §13.B.
- **Clear coat / anisotropy / sheen** (T12): a cold `MaterialXGpu` extension row (the transparency
  design's D2 shape) — coat = a second `eval_pbr_ambient_env` with `f0 = 0.04` and the coat
  normal, base attenuated by `(1 − Fc)`; anisotropy = the bent-R "faux" form (Filament) first,
  BRDF-major-axis sampling later; sheen needs its own Charlie-filtered chain (a second blob per
  environment).
- **Geometric specular AA** (T13): the compute resolve has no `ddx/ddy`; the raster paths do —
  ships on `forward_opaque` / `gbuffer_mrt` first, and on the compute paths when the W3 derivative
  reconstruction lands (`PBR-MATERIALS-PLAN.md:394`).

## 4. eDSL leaves and their gates

Every body is authored ONCE over the scalar/control-flow axes (`boyko_shaderdsl/src/scalar.rs`,
the `Cf` discipline of `oct.rs`), instantiated over `f32` (the oracle) and `Emit`, spliced between
`// === GENERATED <name> BEGIN/END ===` sentinels, pinned by a `<name>_edsl_sync` test that
regenerates and re-DXCs to the committed `.spv` (the `interp_edsl_sync` two-gate shape), with a
manifest row per `-D` variant.

**Which leaves have an oracle, and which cannot** (critique NB7 / B5). The eDSL has two ways to
read memory. `BufferLoad` (`emit/mod.rs:291`) is an ordinary node with an `EvalCf` arm — a body
that reads a `StructuredBuffer` runs on the host, which is why the SDF field has a byte-exact
host mirror. A TEXTURE fetch is a `ResRef` — "the CPU cannot run `atlas.SampleLevel`, so this is
an EMIT-ONLY node (the body is never instantiated over `EvalCf`)" (`:529-535`); the oracle of
such a body receives the fetched value as a parameter (`m2_corner`), which is workable for one
fetch and meaningless for a fetch inside a runtime `Stmt::Loop` (`:2473-2480`) — `cf.rs:730-736`
states the contract for exactly that case: "There is NO eval sweep; the cmp-`.spv` is the SOLE
gate". So every row below is one of three kinds, and the gate column is honest per row:
**oracle** (no fetch, or `BufferLoad` only — `f32` instantiation is the gate), **oracle over
parameters** (one fetch outside any loop — the fetched value is a parameter; the gate covers the
math, a device fixture covers the fetch), **emit-only** (a fetch inside a loop — the cmp-`.spv`
and a device fixture are the whole gate; no host claim is made). Pass 0's "every reflection body
has an `f32` oracle" was the P43 over-claim applied to this design's own code.

| Leaf | Rung | Body | Kind | Oracle / gate |
|---|---|---|---|---|
| `env_lod(pR, L1)` | R0 | `L1·pR·(2−pR)` (Filament's quadratic, tuned for 256/5–7 levels) and its inverse `env_lod_inv(k, L1)` used by the BAKE | oracle | G3: `env_lod(env_lod_inv(k)) == k` for every level; ONE function for bake and read (P1) |
| `oct_env_uv(dir, tile, level)` | R0 | direction → bordered-tile uv (`oct_encode` + the `(tile−2)/tile` interior scale + ½-texel offset) | oracle | G4 seam; the DDGI `ddgi_irr_uv` rule generalised |
| `dfg_integrate(NoV, pR, N)` | R0 | Hammersley GGX, `(scale, bias)` | oracle | `dfg_lut_sync`; G1 furnace |
| `ggx_prefilter_sample(i, N, α, Ωp)` | R0/R3 | VNDF sample + FIS `lod = 0.5·log2(Ωs/Ωp)` | oracle | per-sample direction/lod equality |
| `env_ggx_convolve` | R0/R3 | the accumulate loop: `ggx_prefilter_sample` × `oct_env_uv` × a **`BufferLoad` bilinear** over the packed source level (R3 gate text) | oracle (BufferLoad form) | host bake == GPU chain bit-for-bit under the `sdf_field.hlsli:21-31` contract; firefly bound. A `SampleLevel` `-D` arm, if ever added, is **emit-only** with a per-level tolerance |
| `sh9_project` / `sh9_eval` | R0/R1 | 3-band projection at bake; evaluation at `n` with `max(·, 0)` | oracle | ringing fixture: a single-texel sun ⇒ no negative lobe after clamping |
| `spec_dominant_dir(n, R, pR)` | R1 | Unity's lerp | oracle | oracle equality |
| `eval_pbr_ambient_env` | R1 | §2.1 | oracle over parameters (`env_spec`, `dfg` are fetched by the caller) | G5 0%-gate; **six** splice sites pinned by one census test (§2.1) |
| `parallax_box` / `parallax_sphere` | R2 | Lagarde's AABB / sphere re-aim | oracle | analytic intersection oracle |
| `probe_blend_weight` | R2 | NDF weight + centre/boundary constraints | oracle | the two-probe fixture (R2 gate) |
| `refl_compose` | R2 | §2.3 | oracle over parameters (each source's `(rgb, w)` is a parameter) | G7 seam energy |
| `viewz_from_t` + `hzb_min` reuse | R4b | `t·dot(rd, fwd)`; the `isnan` spelling | oracle | G6 polarity |
| `hiz_reflect_trace` | R5 | T23 traversal — a pyramid fetch per step inside the march loop | **emit-only** | mirror-floor fixture (device); the cell-step arithmetic is split out as `hiz_cell_step`, an oracle'd leaf, so the part that CAN be wrong on the host is |
| `vndf_sample` | R6 | Heitz spherical cap | oracle | pdf integrates to 1 on the host (a count-based check over a sample grid) |
| `refl_resolve_weight` | R6 | the per-neighbour BRDF weight | oracle | variance-reduction gate (the neighbour loop that fetches is **emit-only** and lives in `refl_resolve`, not in this leaf) |
| `virtual_motion` | R7 | hitT-based reprojection, roughness blend | oracle over parameters (`hitT` is a parameter) | a moving-mirror fixture (owner-eval in motion; the static part is a golden) |
| `refl_prefilter` / `refl_temporal` | R7 | 16-tap Halton gather / variance clamp over history | **emit-only** | G8/G9 on the device; the tap weights are the oracle'd leaf `refl_tap_weight` |
| `refl_demodulate` / `refl_remodulate` | R7 | divide / multiply by the split-sum weight | oracle | G9 |
| `sdf_reflect_march` | R8 | the ranged-shadow marcher shape returning hitT + normal over `field_distance` | oracle (the field is `BufferLoad`, `sdf_field.hlsli:13-18`) | SDF mirror golden; host mirror equality (the `golden_composite_pixel_ex` precedent) — the brick-atlas swap (R11) turns this row **emit-only** and says so |

## 5. Gaia and Aether

| Need | Construct | Status |
|---|---|---|
| An environment in a scene | `EnvironmentLight asset="ibl/studio_small" intensity=1.0 yaw=0` on an entity (scene profile) | component authored like `PointLight` (`docs/gaia/LANGUAGE.md:20-40`); the `asset` value is a stable id, never a slot (`DECISIONS.md:96-104`) |
| A probe | `ReflectionProbe shape=box half_ext=(4,3,4) influence=6 blend=0.3 source=baked("ibl/crypt_p01")` + `Transform` | same |
| The environment asset | a `data`-profile document naming the foreign `.hdr` (sidecar id, hard fail on absence — `DECISIONS.md:123-127` applies to `.png`/`.glb` today; `.hdr` joins that list) + bake settings (`tile`, `levels`, `sun_baked`, exposure pre-scale, `up_axis`) | the bake is eager/closed; the P10 traps (orientation, up-axis, exposure) are **bake settings with no default**, so a missing one is a bake error, not a guess |
| The DFG LUT | not a Gaia asset — a committed `.bin` regenerated by a test (§1.2) | — |
| Logic | none — an Aether `system` may write `ReflectionConfig`; no new construct | — |

**New constructs for v1: none.** Two format requests: AK-6 (the asset-blob region + blit into an
append-only column — shared with animation, §11) and a `.hdr` sidecar-id rule (the existing
non-Gaia binary rule extended by one extension).

## 6. Storage forks decided, and the decoder gap

### 6.1 Octahedral vs cube (the largest architectural fork)

| Axis | Cube views | Octahedral 2D / `D2Array` |
|---|---|---|
| RHI growth | `TextureViewDimension::{Cube, CubeArray}`, `VK_IMAGE_CREATE_CUBE_COMPATIBLE` at `texture.rs:240-244` (only `MUTABLE_FORMAT` is chosen today), 6-layer create path, `TextureCube` HLSL declarations, `abi_guard` rows | **zero** — `D2` and `D2Array` exist (`enums.rs:526-542`) |
| Seams | hardware seamless filtering (free) | 1-texel border per mip, written by the convolution (§1.4) — the DDGI rule, already oracle'd |
| Sample-density uniformity | 6 faces, corner texels stretched | more uniform (Engelhardt & Dachsbacher [D]); a 256² oct ≈ a 104-px-face cube in texel count |
| eDSL | new `TextureCube` node kind | `oct_encode`/`oct_decode` leaves exist with oracles |
| Probe arrays | `CubeArray` + `MAX_TEXTURE_LAYERS` × 6 | `D2Array`, one layer per probe |
| Bindless | table is `Texture2D[]` — a second table | fits the existing table shape if ever needed |
| Precedent | UE, Bevy (binding arrays — with a portability cliff, P-class) | HDRP 14 (2D octahedral atlas), this tree's DDGI |

**Decided: octahedral** (D1). The cube road adds four RHI items and a shader node kind to buy
seam handling the octahedral road already pays for with an oracle'd border rule.

### 6.2 The decoder gap — closed in-house, RGBE first

`boyko_image` decodes PNG only (`lib.rs:9-19`). The environment source path is:

1. **Radiance RGBE `.hdr`** (R0): ASCII header (`#?RADIANCE`, `FORMAT=32-bit_rle_rgbe`), the
   `-Y M +X N` resolution line, then per-scanline either flat RGBE quads, old-style RLE, or the
   new-style RLE flagged by the `(2, 2, hi, lo)` marker with four per-component runs (T15, [D]
   LBL `picture_format`). Decode to `f32` RGB (`(m + 0.5)·2^(e−136)` per channel, `0` when `e ==
   0`). **~200 lines, `std` only**, the crate's own discipline ("zero third-party dependencies").
   Gate: the crate's spec-text fixtures (a hand-built 4×2 image in each of the three scanline
   forms, byte-exact) + a round trip through the writer the bake tool needs anyway.
2. **16-bit PNG** as an LDR sky source — already decoded; the baker treats it as `[0, 1]` linear
   after the sRGB decode the asset declares.
3. **Not** EXR (a real codec: zip/piz/pxr24), **not** KTX2 + BC6H (needs a BCn `Format` family,
   `abi_guard` rows, a container reader and an offline encoder — the deferred
   `docs/PBR-TEXTURES-PLAN.md` that does not exist). Both are recorded as later doors; BC6H's
   4× VRAM saving matters only when environments are many and large, which §1.4's arithmetic
   says they are not.

Equirect → octahedral resampling is a host step of the bake (bilinear over the equirect,
oct-decoded direction per texel, then the prefilter), so the runtime never sees an equirect.

## 7. Participation matrix

| Consumer | Deferred | Forward | ForwardPlus | VB (fused) | VB (split) | SDF leg | Transparent (transparency campaign) |
|---|---|---|---|---|---|---|---|
| R1 env + LUT + SH | `deferred_pbr` | `forward_opaque.fs` (base compile) | `forward_opaque.fs` (`-D FROXEL=1`) | `vb_resolve` / `vb_shade` (classified) | `vb_shade_split` | `sdf_forward_march` (Forward, VB); the resolve (Deferred) | `forward_translucent.fs` |
| R2 probes | froxel list (L1 block) | **flat walk** over `[l0a_count, light_count)` — bounded by `MAX_LIGHTS`, and it IS P33 on this compile | froxel list (`ClusterGrid`/`LightIndexList`, `forward_opaque.fs.hlsl:16-21`, `:128`) | **flat walk** (`vb_resolve.comp.hlsl:5`: "ALL-LIGHTS, no froxel — mirrors plain `Forward`") | **flat walk** | **flat walk** (`sdf_forward_march.comp.hlsl:37`) | ✔ (its D8 arms the cull) |
| R4b pyramid | `viewt_from_depth` (mesh) / marcher (SDF) | **no producer today** → `fwd_viewt` + marcher-VIEWT, RK-12 | same as Forward | `vb_viewt` (mesh) / marcher-VIEWT (SDF) | `vb_viewt` | marcher-VIEWT where the path declares it (Deferred, VB; Forward after RK-12) | reads, never writes |
| R5–R7 SSR | ✔ | ✔ (after RK-12) | ✔ (after RK-12) | ✔ (after `vb_geo`) | ✔ | ✔ | front layer only (R12) |
| R8 SDF trace | SDF leg | SDF leg | SDF leg | SDF leg | SDF leg | ✔ | — |
| R10 HW trace | mesh leg, hwrt | mesh leg, hwrt | mesh leg, hwrt | mesh leg, hwrt | mesh leg, hwrt | — | — |

**D6's "no linear search per fragment" is true on the froxel compiles and false on the flat ones**
(critique NB1). Plain `Forward` is the ALL-LIGHTS compile — only `-D FROXEL=1` walks the cluster
lists (`forward_opaque.fs.hlsl:16-21`) — and the three VB producers plus the SDF marcher are ALL-
LIGHTS by their own headers. On those compiles a probe row costs one kind-compare per light row
per fragment, bounded by `MAX_LIGHTS`; with ≤ 64 probes that is the cost the point/spot rows
already pay there, so the design accepts it on the flat compiles and does not pretend it away —
D6 now says "no linear search where a froxel list exists". The `vb_froxel` cull that exists for
VB (`vb_froxel_spv` in the sync-test list, RESEARCH §8) makes a `-D FROXEL=1` compile of the VB
producers the door to closing it there; that is not in this campaign.

## 8. HDR scene colour — the decision and its four costs

`lit` becomes `B10G11R11_UFLOAT` (D4) **where the device supports it as a storage image and a
colour attachment (RK-14), else `R16G16B16A16_SFLOAT`**: 4 B/px — the same bandwidth as today's
RGBA8; unsigned float with 5–6-bit mantissas, which is the precision every reflection SOURCE needs
(DDGI already stores radiance in it "bit-exact", `enums.rs:365-370`) while the ACCUMULATORS
(`taa_hist`, `refl_hist`) stay RGBA16F. `R16G16B16A16_SFLOAT` (8 B/px) is the DEFAULT alternative
rejected on bandwidth — every opaque producer and every consumer of `lit` doubles its traffic for a
mantissa the sources do not need — and the DEGRADE arm on a device without
`shaderStorageImageExtendedFormats` (`device.rs:256-264`), where booting at 2× bandwidth beats not
booting (critique B2: pass 0 rejected RGBA16F outright and left that device with no outcome).
Alpha is lost — the transparency design does not read `lit.a` (its §14 says so) and uses its own
`taa_cov`. The four costs, stated with the transparency campaign's R11: the tonemap + OETF leave
**eight** producers (the R4a list; `forward_sky`, `sdf_forward_march`, `vb_resolve` and
`ssaa_downsample` were missing from pass 0's "three") for one `tonemap` pass; TAA moves
pre-tonemap with luma weighting; the additive-blend proof at `enums.rs:938-944` is rewritten for
float saturation; **all 30 pins re-bless** — with the per-producer transition pins of R4a so the
re-bless is verified against the old image rather than accepted.

## 9. Cost model (estimates; the published anchors are RESEARCH §4)

| Rung | Estimate @1080p, RTX-3060 class | Basis |
|---|---|---|
| R1 | 0.05–0.15 ms | bandwidth arithmetic, §3 |
| R2 | +0.05–0.2 ms | ≤ 4 fetches for the covered fraction |
| R3 (per-frame filter) | 0.3–0.8 ms, or ~0.1 ms amortised over 8 frames | Bevy's sample schedule × 256² |
| R4a | ≈ 0.05 ms + 8 MB copy | one fullscreen pass |
| R4b | ≈ 0.1 ms | SSSR SPD 0.12 ms (3080 Mobile) |
| R5 | 0.5–1.0 ms | SSSR intersect 0.88 ms full-rate; half-res, roughness-gated |
| R6 | ≈ 1.5 ms cumulative pre-denoise | SSSR classify + intersect + prefilter |
| R7 | 1.5–2 ms | SSSR DNSR 1.53 ms (57 % of 2.70) |
| R8 | 0.5–1.5 ms for a 20 % miss fraction | `SDF-PERF-AUDIT.md:82-85` evals/pixel |
| R9 | ≈ 0 | a sample already paid |
| R10 | unmeasured | no published figure for this class |
| **Full ladder** | **≈ 4–6 ms** | sum; the owner's frame budget decides the default config (§13.B) |

Memory: §1.4 + §1.5 ≈ **80 MB** at R7, 12 GB available on the box; the FIF ringing is counted.

## 10. Gates (all red-first)

- **G1 furnace** (host, R0): white environment, `f0 = 1`, sweeps of `pR` and `NoV` →
  `spec_ambient` within `[−2 %, +0.5 %]` of 1 with LUT + compensation; the analytic fit fails the
  same sweep at `pR > 0.7`, `NoV < 0.2` (the P8 amplification, quantified once).
- **G2 LUT sync**: `dfg_integrate` regenerates `DfgLut.bin` byte-for-byte.
- **G3 bake/read inverse**: `env_lod ∘ env_lod_inv == id` per level; on the device, a chrome
  `pR` sweep reads the predicted mip (the Godot #69514 fixture, red on a mismatched pair).
- **G4 seam**: an oct fetch ½ texel outside the interior == the fetch at the mirrored direction
  within `CHANNEL_TOL`, at every mip.
- **G5 0%-gate**: no `EnvironmentLight`, no `ReflectionProbe`, `SsrMode::Off` ⇒ every existing
  pin byte-identical (30, `FEATURE_MAP.md:1059`) — until R4a, which is a declared re-bless with
  its own transition pin.
- **G6 polarity**: the pyramid's level-k texel == host `min` over view-z; the `max` variant red.
- **G7 seam energy**: confidence 0 ⇒ `refl_compose` == the R1 answer bit-for-bit; roughness at
  `max_trace_roughness ± ε` ⇒ no step larger than `CHANNEL_TOL`.
- **G8 accumulator separation**: TAA off/on ⇒ `refl_color` identical.
- **G9 demodulation identity**: constant environment ⇒ texture-free denoised reflection.
- **G10 variant census**: every `.spv` in `SHADER-VARIANT-MANIFEST.md`, every leaf has an
  `_edsl_sync`, no `OpFMin` in any reflection pyramid `.spv`.
- **G11 counts, not clocks**: the classifier's traced-pixel count, the probe rows per cluster,
  the evaluations per rung are asserted as counts (`docs/gaia/DECISIONS.md:282-289`); every
  timing in §9 is re-measured on the owner's box **only on the owner's word**
  (`feedback-ask-before-loading-the-machine`).

## 11. Kernel and crate requests born from the design (each gets its own pass)

| # | Item | Why | Zero when unused |
|---|---|---|---|
| RK-1 | `boyko_rhi`: `MipMode::Linear` (trilinear) on `SamplerDesc` (`crates/boyko_rhi/src/device.rs:241-245`) | the only trilinear sampler is built below the RHI (`crates/boyko_rhi_vulkan/src/bindless.rs:139-158`); an env chain sampled at level 0 is the P1 silent wrong | yes |
| RK-2 | `boyko_render::upload_texture_2d_raw`: an `R16G16Sfloat` (4 bpp) arm (`texture.rs:381-386`) — **folded into RK-11** | the DFG LUT | yes |
| RK-3 | `boyko_rhi_vulkan`: per-layer render views only for attachment usage; `MAX_TEXTURE_LAYERS` (`texture.rs:117`) lifted to 64 for sampled/storage arrays | > 16 probes per atlas | yes |
| RK-4 | `boyko_image::decode_hdr` (RGBE) | §6.2 | yes |
| RK-5 | `boyko_serialize` / Gaia: the asset-blob region + blit into append-only asset columns — **AK-6 of the animation design, the same request** | §1.1 | v2 files unchanged |
| RK-6 | light table: `LIGHT_KIND_PROBE = 4` + the parallel `ProbeGpu` SSBO; the three kind predicates of `cluster_cull.hlsl` (`:324`, `:357`, `:476`) widened to admit it; the probe span written LAST in L0a by `fold_light_table`/`_slotted` (`light_system.rs:225`, `:273`) and **insertion-sorted in place in `dst`** by (priority, volume) — §1.3; the `EnvironmentLight` gather writing the single SKY row (D20) | §1.3, §1.6 | no rows ⇒ byte-identical (the sort runs over an empty span) |
| RK-11 | `boyko_render::upload_texture_2d_raw_mipped(ctx, w, h, levels, bytes, format)`: `levels` buffer→image copies at `BufferImageCopy.mip_level = k` (`crates/boyko_rhi/src/descriptor.rs:628`), level-major offsets, no blit; formats `R16G16Sfloat` (4) and `B10G11R11UfloatPack32` (4) added to the bpp match | R1's host-baked chain cannot be uploaded by anything in the tree (`texture.rs:164-168` blits, `:373-386`/`:403` is single-level) — critique B3 | yes |
| RK-12 | `graph_bridge.rs::declare_forward_graph` (`:2585`): a `fwd_viewt` pass (the `viewt_from_depth_rz` body over Forward's reverse-Z `depth`) and the VIEWT-variant marcher's `viewt` access, both armed by the pre-light union; the tripwire at `:2936-2941` becomes the arming check | Forward/ForwardPlus have no `gViewT` producer (§2.2) — critique B4 | yes (unarmed ⇒ no pass) |
| RK-13 | `boyko_render::taa_jitter::JitterState`: a `prev_phase`/`prev_armed` pair kept by `advance_jitter`; `ndc_jitter_prev` | the trace subtracts the previous frame's jitter at the `lit_prev` fetch (D19) — critique NB4 | yes (`{0, 0}` when unarmed) |
| RK-14 | `boyko_rhi_vulkan::DeviceCaps::lit_hdr_format_ok`: B10G11R11 `STORAGE_IMAGE` + `COLOR_ATTACHMENT` under OPTIMAL (the `viewt_storage_format_ok` probe shape, `device.rs:3266`, two bits); `false` ⇒ `lit`/`lit_prev`/SSAA ring are `R16G16B16A16_SFLOAT` | `lit` is a storage image on six producers and an attachment on Forward; B10G11R11 storage is optional (`device.rs:256-264`) — critique B2 | R4a only |
| RK-7 | `graph_bridge.rs`: the reflection image block appended last on both legs; `FRAMEGRAPH_IMAGE_COUNT` per rung | §1.5 | — |
| RK-8 | the per-view seam (MPR R11) for captured probes and planar | R3, R12 | — |
| RK-9 | a `.hdr` sidecar-id rule (the `.png`/`.glb` rule, `DECISIONS.md:123-127`, extended) | §5 | — |
| RK-10 | render: per-object motion un-walled from `hwrt` (shared with animation's prev-position ring) | R7 on moving reflectors | — |

## 12. Landing order and byte-identity accounting

```
R0  decoder + LUT + host bake       no shader change; 0 pins move
R1  env chain into the ambient site 0 pins move (G5); +1 armed golden
R2  probes                          0 pins move; +1 armed golden
R4a HDR lit + tonemap tail          ALL 30 pins re-bless (declared; transition pin verifies)  ← shared with transparency R11
R4b pyramid                         0 pins move (unarmed) ; +1 oracle gate
R5  mirror SSR                      0 pins move (unarmed); 4 `ssr_on` assertions re-blessed; +1 golden
R3  GPU prefilter + captured probes 0 pins move; equality-to-bake gate
R6  stochastic                      0 pins move; +variance gate
R7  denoiser                        0 pins move; G8/G9
R8  SDF trace                       0 pins move; +SDF mirror golden + cost bench in gate
R9  DDGI tail                       0 pins move (DDGI default off)
R10 HW trace                        owner-eval
R11 / R12                           their own passes
```

R3 is placed after R5 deliberately: a static baked environment is what R1 needs, and the GPU
filter's first real consumer is the captured probe, which needs the per-view seam that R5 does
not. Every rung is a **new pass or a new column, never a new path** (the transparency design's
rule, kept).

## 13. Decisions

### 13.A PERF / ARCHITECTURE — taken here, with the numbers

| # | Decision | The number that decides it |
|---|---|---|
| D1 | **Octahedral** environment and probe atlas, not cube views | 0 RHI items vs 4 + a shader node kind (§6.1); `oct_encode`/`oct_decode` exist with oracles; DDGI's bordered-tile rule already oracle'd; HDRP 14 made the same move |
| D2 | Borders written by the convolution on every mip; the chain stops at 4×4; **no blit/average mips anywhere** | P5: auto-mips pull gutters inward; a 256 tile gives 7 usable levels, the regime Filament's quadratic map is tuned for (T4) |
| D3 | ONE `env_lod` leaf for bake and read; Filament's quadratic form | P1: three engines, three mappings, one shipped mismatch (Godot #69514) |
| D4 | `lit` → **B10G11R11 when `lit_hdr_format_ok` (RK-14), else R16G16B16A16_SFLOAT**; accumulators stay RGBA16F | 4 B/px = bandwidth parity with RGBA8 **on the probed arm**; RGBA16F doubles every producer's traffic for mantissa the sources do not need, and is the degrade arm rather than a fail-fast (the `atlas_linear_filter_ok` rule, `device.rs:250-254`); DDGI stores radiance in it bit-exact today |
| D5 | The DFG LUT is a committed `.bin` regenerated by a test; energy compensation keeps its form, changes its input | the SMAA shape exists (`smaa_luts.rs`); P8 is fixed by the input, not the formula; the Filament channel convention is not depended on (§14) |
| D6 | Probes are **light-table rows** clustered by the froxel cull; ≤ 4 blended; size-ascending order made by an in-place insertion sort in the fold (§1.3) | no linear search per fragment **where a froxel list exists** (Deferred, ForwardPlus); on the ALL-LIGHTS compiles (plain Forward, the three VB producers, the SDF marcher) the walk is flat and bounded by `MAX_LIGHTS` — stated per path in §7 (critique NB1); `MAX_LIGHTS_PER_CLUSTER = 256` bounds the clustered walk; the 0%-gate is "no rows" |
| D7 | The reflection pyramid is **view-z from `gViewT`**, `min`-reduced, on every path **once every path has a `gViewT` producer (RK-12 for Forward/ForwardPlus)**; a separate image from the occlusion HZB | `gViewT` is the input SSAO/SSCS/TAA already share, so one builder serves four paths and two depth kinds (`DepthKind::CustomLinear` vs reverse-Z, `render_path_config.rs:419-431`); `min(z)` = nearest with no polarity argument (P16); the Forward producer is the `viewt_from_depth_rz` body, whose decode already matches Forward's encode (§2.2) |
| D8 | SSR reads **`lit_prev`** (previous frame, HDR), never the current frame | it is the only order that works under the VB split (no colour before `vb_shade`, P38) and it is Godot/Filament/HDRP's choice (T29); the copy is shared with the transparency campaign's `scene_color_copy` |
| D9 | Half-res trace + ratio-estimator resolve from R6; mirror-only at full confidence in R5 | every shipping system traces at half/quarter (P26); SSSR's 0.88 ms intersect is the anchor |
| D10 | The denoiser is a **new** three-pass radiance denoiser with its own history and dual reprojection; the shadow denoiser is a structural template only | P19/P27: scalar vs 3-channel HDR, hit distance, virtual motion; SSSR's denoiser is 57 % of its total (P22) — budgeted, not assumed |
| D11 | Roughness gate at 0.4 for tracing; 0.7 for the DDGI tail; smooth ramps at both | Lumen's single biggest saving (P25); Godot's 0.7 skip; P32 needs a ramp, not a step |
| D12 | Sources compose under ONE split-sum weight applied after `refl_compose` | P32: energy match at every seam is free when the weight is applied once |
| D13 | Baked environments prefiltered on the **host** by the leaf's `f32` instantiation; the GPU filter (R3) must equal it **bit-for-bit, which is attainable only because the convolution reads its source through `BufferLoad` and filters in the body (R3 gate)** | the bake is its own oracle for the WHOLE loop only in that form — a `SampleLevel` fetch is emit-only and would cap the oracle at the per-sample math (critique B5); zero runtime cost for the static case; the GPU chain's gate is equality, not a golden |
| D14 | The sun disc is retired by a bake flag, not deleted | P11: triple count when the map holds the sun; the 0%-gate needs the disc for the analytic mode |
| D15 | Blue noise is a baked 128² Owen-Sobol tile; IGN is the interim only | T40: white/IGN converges visibly slower under temporal accumulation; NRD's stated floor is 64² |
| D16 | Every reflection body is an eDSL leaf spliced into the existing files; the hand BRDF core is untouched; **each leaf is labelled oracle / oracle-over-parameters / emit-only in §4** | RESEARCH §0.1 / P43: the BRDF has no `_edsl_sync` tripwire, so the new code must bring its own; a leaf that fetches inside a loop has no host oracle by the eDSL's own contract (`cf.rs:730-736`), and saying otherwise is P43 turned inward (critique NB7) |
| D19 | `lit_prev` is copied BEFORE `taa_resolve` and the trace subtracts the previous frame's `ndc_jitter` at the fetch (RK-13) | P17/P27 independence from TAA is kept; `MotionCam` is jitter-free by construction (`taa_resolve.comp.hlsl:21`) so the raw fetch is up to ½ px off and phase-dependent; two floats fix it, a post-TAA copy would couple two histories (critique NB4) |
| D20 | The `EnvironmentLight` gather writes the ONE `LIGHT_KIND_SKY` row (sky/ground lanes from the SH DC term) and sets word-7 bit 7; a co-present `SkyLight` writes no row | the ambient call is reachable only inside the SKY branch at all six sites (`deferred_pbr.hlsl:1151-1163`); moving the call would re-order `ambient` at six sites and break G5 (critique NB3) |
| D21 | Diffuse composition: `diffuse_color · (gi + (1 − w_gi) · sh_diffuse) · ao_final`, `w_gi` = the DDGI coverage/convergence weight, 0 structurally when DDGI is off | DDGI is additive today over a guessed base (`deferred_pbr.hlsl:1178-1181`, `:1195`); over a real sky SH that is a double count of the sky inside the grid — P11's diffuse twin (critique NB2) |
| D17 | RGBE decoder in-house; EXR and KTX2/BC6H are recorded doors, not rungs | ~200 lines vs a codec + a `Format` family; §1.4's sizes make BC6H's saving immaterial today |
| D18 | Probe capture through the per-view seam, 6 raster faces → cube→oct resample on the GPU; no octahedral raster | a raster cannot draw into an oct parameterisation; the SDF leg captures directly by marching (the DDGI probe-update shape) |

### 13.B VALUES / SCOPE — to the owner

1. **HDR scene colour (R4a)** — now, before R5, or never: re-blesses all 30 pins, moves TAA
   pre-tonemap, rewrites the additive proof. The same ballot as the transparency campaign's
   ballot 1; recommended **before R5** (nothing screen-space is right without it, P15).
2. **The default configuration and its frame budget**: the full ladder is ≈ 4–6 ms (estimate,
   §9); which rungs are on by default in the showcase is a budget call.
3. **Environment map resolution** (256 default; 512 costs 4× and adds one level) and the probe
   tile (128); **B10G11R11 vs RGBA16F** for the atlases (precision vs 2× memory).
4. **Dynamic sky cadence** (R3): every frame (0.3–0.8 ms) or amortised with a stated lag.
5. **Probe budget** (16 until RK-3; 64 after) and whether runtime capture (R3) is in this
   campaign or its own.
6. **`max_trace_roughness`** = 0.4 (Lumen) as the default, and the DDGI-tail cutoff 0.7.
7. **HW-RT reflections (R10)** — owner-eval only, untestable in CI on this box; in scope or
   parked with the rest of the hwrt track.
8. **R11 (radiance atlas + cone trace / Radiance Cascades)** — planned now or left as the
   research rung.
9. **Planar reflections** — a second full view; UE's "half your frame time".
10. **Material extensions** (clear coat / anisotropy / sheen, R12) — the cold extension table is
    shared with transparency's D2; which lobes are v1.
11. **The sun in the environment**: bake it (retire the disc) or keep the analytic disc + a
    sun-less map; the P11 triple count is the reason it cannot be both.
12. **`.hdr` authoring conventions** (up-axis, exposure pre-scale) as **mandatory bake settings**
    with no default (recommended — the P10 trap class) vs defaults.

## 14. Unverified / open

- Every millisecond in §3/§9 is either a published figure on another GPU (RESEARCH §4 names each
  rig) or bandwidth/count arithmetic labelled as an estimate; nothing was timed on this box.
- Filament's multiscatter-LUT channel convention (whether `dfg.y` holds `Ess` or the bias) was
  not confirmed from `Filament.md.html` (RESEARCH §7); D5 defines the engine's own channels so
  nothing rests on it.
- Whether `sscs_march` (`deferred_pbr.hlsl:691`) compares `view_z` or `t` was not re-read for
  this document; D7 states its own measure (view-z) and does not inherit SSCS's.
- The cull's bin test and its three kind predicates were read (`cluster_cull.hlsl:322-326`,
  `:357`, `:476`), and — after critique NB5 — so was the list writer (`:342-374`, ascending
  index order, no sort) and the Rust fold's row loops (`light_system.rs:290-305`); RK-6 is now
  sized with the in-place sort (§1.3). The `GpuLight` field order itself is still only cited by
  its constructors (`from_directional`, `from_sky`), not re-read field by field.
- **Two ladder candidates the critic could not source, recorded as UNVERIFIED, not as rungs**
  (critique NB11): (a) reflection-capture **normalisation** against baked / probe-volume lighting
  (UE's "reflection capture normalization", HDRP's "reflection probe normalization with APV") and
  (b) HDRP **sky occlusion** for probe lighting. The three pages the critic opened —
  `dev.epicgames.com/documentation/en-us/unreal-engine/reflections-environment-in-unreal-engine`,
  `docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Reflection-Probe.html`,
  `…/probevolumes.html` — carry no normalisation text and only a table-of-contents mention of sky
  occlusion. If (a) is real it is a term applied where R2's probe meets R9's DDGI (the probe's
  baked ambient divided out and the live irradiance multiplied in, so a probe does not carry
  stale sky), and it would sit between R2 and R9 as an R2b; until a primary source is read it is
  neither ruled in nor out. **§7 of the RESEARCH** ("What the research could not resolve") carries
  the same entry — pass 1 cited a "§16" of that document, which does not exist (it ends at §10).
- The Vulkan mandatory-format table for `B10G11R11_UFLOAT_PACK32` did not serve for this
  revision (`docs.vulkan.org/spec/latest/chapters/formats.html` returned the format list, not the
  "Required Format Support" tables), so RK-14 probes BOTH the storage and the colour-attachment
  bits rather than assuming either; a later pass that reads the table may drop the attachment
  bit from the probe if the spec makes it mandatory.
- The RK-13 previous-jitter rule assumes `lit_prev[1 − fi]` was produced exactly one frame ago;
  on a frame after a TAA reset (`taa_state.rs`) the previous frame may have been unarmed, which
  `prev_armed = false` handles as an exact-zero jitter — the case of a resize (both rings
  recreated) is a first-frame miss the confidence already encodes (`refl_hist.a == 0`).
- The per-view seam (MPR R11) is "optional … ship only if per-view adds no allocation"
  (`MULTI-PARADIGM-RENDER-PLAN.md:497`); R3/R12 make it required, and its allocation profile is
  unmeasured.
- The `refl_hiz` seed choice (explicit `undefined` vs a cross-frame seed) depends on whether any
  pass reads the pyramid while the sibling frame writes it; R5 reads it in the same frame only,
  so `undefined` is expected — stated as a decision at the declare site, per `framegraph/graph.rs:383-385`.
- Lagarde's K-th-contributor popping (P34) is mitigated by the blend width, not solved; no
  surveyed engine solves it.
- Whether B10G11R11's 5-bit mantissa is enough for `lit_prev` under the ratio-estimator resolve
  (a divide by a near-zero weight amplifies quantisation) is a fixture question at R6; the
  fallback is an RGBA16F `lit_prev` at +8 MB.
- The R8 miss fraction (20 %) is a guess; the cost bench in the gate replaces it.
- No RTX 3060 per-pass reflection figure exists in public (RESEARCH §7).

## 15. Critique log — pass 1 (2026-09-10)

Each finding of the `architecture-critic`'s first pass, with what this revision did about it.
"Fixed" means the text above now says something different; "refuted" means the finding's
evidence was re-opened and does not support it; "partly" means part of the chain was wrong and
the rest was fixed. Every `file:line` here was re-opened at commit `ed0bed45` by the architect;
the critic's censuses were re-run, not copied. **Three rows of this table were nevertheless wrong when written — see §15.1, which audits the log itself; read the two together, never this table alone.**

| # | Finding (short) | Action |
|---|---|---|
| B1 | §2.1 names three splice sites + `vb_shade_split` and "two verbatim duplicates"; the tree has SIX `eval_pbr_ambient_hemi` call sites and SIX `env_brdf_approx` definitions — `vb_resolve` (the fused VB path) and `sdf_forward_march` (the SDF leg) are omitted, so under `env_mode == 1` two paths keep the gradient while the gate is green | **Fixed.** Re-grepped: calls at `deferred_pbr.hlsl:1163`, `forward_opaque.fs.hlsl:303`, `sdf_forward_march.comp.hlsl:1080`, `vb_resolve.comp.hlsl:368`, `vb_shade.comp.hlsl:522`, `vb_shade_split.comp.hlsl:540`; definitions at `:582`, `:182`, `:855`, `:212`, `:247`, `:308`. §2.1 is now a six-row table with each file's path role (`vb_resolve.comp.hlsl:1-8`, `:23`; `sdf_forward_march.comp.hlsl:37-38`; `vb_shade.comp.hlsl:11-14`), and the pin is a **census test** (call-site count == sentinel count) rather than a list, so a seventh producer cannot be missed the same way. §4, R2 and §7 updated to six. ~~RESEARCH §0.2 corrected (its C12)~~ — **that half was asserted, not done; see §15.1. Landed in pass 2** |
| B2 | R4a re-types `lit` to B10G11R11 with "no technical prerequisites"; `lit` is a STORAGE image and B10G11R11 storage is device-OPTIONAL (`shaderStorageImageExtendedFormats`), probed and degraded on for DDGI only; §8 rejects RGBA16F, so a device without the feature has no boot outcome, and D4's parity is conditional on an unchecked feature | **Fixed.** `targets.rs:123-127` (STORAGE ring), `:7236-7252` + `:7695`/`:8885` (`ImageUsage::STORAGE`), `:782-784` (Forward colour attachment), `device.rs:256-264` re-opened. **RK-14** `lit_hdr_format_ok` probes storage + attachment; `false` ⇒ `lit`/`lit_prev`/SSAA ring are `R16G16B16A16_SFLOAT` (core-mandatory storage, `enums.rs:362-364`) — a degrade, never a fail-fast. R4a prerequisites, gate (fallback arm exercised via the `DeviceCaps` test constructor, `device.rs:4107`), cost, §1.5, §8 and D4 all say "parity on the probed arm". The attachment bit is probed rather than assumed because the spec table did not serve (§14) |
| B3 | R1 uploads the chain "from the `EnvAsset` blob", but D2 forbids blit/average mips and no upload path can fill a host-baked chain: `upload_texture_2d_raw` is single-level, `upload_texture_2d` blits; RK-2 asks only for a bpp arm | **Fixed.** `texture.rs:164-168`, `:373-386`, `:403` re-opened; the primitive exists at `descriptor.rs:628` (`BufferImageCopy.mip_level`) and the level-0 copy at `texture.rs:246-262`. **RK-11** `upload_texture_2d_raw_mipped` (N copies at `mip_level = k`, no blit) added; RK-2 folded into it; R1 prerequisites and §1.4 name it |
| B4 | §2.2/D7/R4b rest on "`gViewT` exists on every path"; the Forward declarator declares no `viewt` write and asserts it; the two producers are Deferred×Mesh (`viewt_from_depth`) and VB (`vb_viewt`); the §7 row is wrong | **Fixed.** `graph_bridge.rs:2936-2941`, `:1319-1330`, `:3608-3609`, `forward_opaque.vs.hlsl:16-17`, `viewt_from_depth_rz.comp.hlsl:5-12` re-opened; an awk over `:2585-3300` finds `viewt` only in the tripwire and the ResId list at `:3110`. **RK-12** (a `fwd_viewt` pass = the `viewt_from_depth_rz` body over Forward's reverse-Z depth + the marcher-VIEWT access) added; §2.2, §2.4, R4b prerequisites, §7 and D7 now state per path which producer exists and that Forward's is absent until RK-12 |
| B5 | R3's gate — GPU chain == host bake bit-for-bit / 1 ULP — cannot pass over a trilinear `SampleLevel`; DDGI is tolerance-gated for that reason; attainable only if the convolution reads with Load and filters in the leaf; D13's "bake is its own oracle" is capped by `ResRef` being emit-only | **Fixed.** `ddgi_probe_gi_sync.rs:123`, `emit/mod.rs:529-535`, `:291` (`BufferLoad` has an `EvalCf` arm), `cf.rs:730-736`, `sdf_field.hlsli:13-31` re-opened. The R3 gate is rewritten: `env_ggx_convolve` reads the source level as a `StructuredBuffer<uint>` through `BufferLoad` and does its own bilinear/trilinear in the body under the `sdf_field` determinism contract, so the whole loop is one `f32`-instantiable body and bit-exact is real; a `SampleLevel` arm is permitted only as a `-D` variant with a stated tolerance. D13 and the §4 row say so |
| NB1 | §7 marks Forward "✔ (froxel lists)" and D6 claims "no linear search per fragment" on every path; plain Forward is ALL-LIGHTS, only `-D FROXEL=1` walks clusters; the VB producers and the SDF marcher are ALL-LIGHTS too | **Fixed.** `forward_opaque.fs.hlsl:16-21`, `:128`; `vb_resolve.comp.hlsl:5`; `sdf_forward_march.comp.hlsl:37` re-opened. §7 splits Forward from ForwardPlus and marks the flat walks per producer; D6 is "no linear search where a froxel list exists"; the bound (`MAX_LIGHTS`) and the fact that it IS P33 there are stated, not avoided |
| NB2 | §2.3 composes only specular; DDGI is added on top of the hemisphere today, and once R1 is a real sky SH a receiver inside the grid counts the sky twice — the diffuse analogue of P11 | **Fixed.** `deferred_pbr.hlsl:1178-1181`, `:1189-1195` re-opened. **D21**: `diffuse_color · (gi + (1 − w_gi) · sh_diffuse) · ao_final` with `w_gi` the DDGI coverage/convergence weight (0 structurally when off); §2.3 carries the rule and its furnace gate; R9 owns `w_gi` |
| NB3 | The ambient call is inside the loop's `LIGHT_KIND_SKY` branch; §1.6 says `EnvironmentLight` is "gathered like `SkyLight`" but not that it EMITS a SKY row — a scene with only an `EnvironmentLight` never reaches the call | **Fixed.** `pbr_lighting.hlsli:152-153`, `deferred_pbr.hlsl:1151-1163`, `light_system.rs:184`, `:228`, `:297-305` re-opened. **D20**: the `EnvironmentLight` gather writes the single SKY row (sky/ground lanes from the SH DC term) and sets bit 7; a co-present `SkyLight` writes no row; gate stated. Moving the call out of the loop was considered and rejected (six-site re-order, G5) |
| NB4 | `lit_prev` is copied before `taa_resolve` (jittered) but reprojected through the zero-jitter `MotionCam` → up to ½ px off, shimmering with the phase; pick subtract-prev-jitter or copy post-TAA and say why | **Fixed.** `taa_resolve.comp.hlsl:13-21`, `runner.rs:43-44`, `:1544-1552`, `taa_jitter.rs:69-73` re-opened. **D19** subtracts the previous frame's `ndc_jitter` at the fetch via **RK-13** (`prev_phase`/`prev_armed` on `JitterState`); post-TAA copy rejected for the P27 history coupling; G8 gains a phase-invariance fixture |
| NB5 | "Rows are written size-ascending" — the cull preserves table order, so ordering is the Rust writer's job; `fold_light_table` takes iterators with no `Vec`; a sort needs a site and an ECS-native store; §14 admits the writer was not read | **Fixed.** `cluster_cull.hlsl:342-374`, `light_system.rs:179-181`, `:225`, `:273`, `:290-305` re-opened. §1.3 names the site — the probe span written last in L0a and **insertion-sorted in place in `dst`** (no side array, no allocation, cold) — RK-6 re-sized; the §14 admission replaced by what was read and what still was not (the `GpuLight` field order) |
| NB6 | §8/R4a count "three producers" of the tonemap+OETF; eight shaders apply it, incl. `forward_sky`, `sdf_forward_march`, `vb_resolve`, `ssaa_downsample` | **Fixed.** Re-grepped: eight files + the `pbr_lighting.hlsli:181-209` definition. R4a lists all eight, gives the SSAA leg its new shape (HDR box filter, then `tonemap` — closing the `avg(tonemap(x))` ceiling its header records at `:17-19`), and runs one transition pin **per producer**; §8 says eight |
| NB7 | D16/§0 "every reflection body is an eDSL leaf with an `f32` oracle" over-claims: a fetch inside `Stmt::Loop` is emit-only, so the trace/prefilter/temporal/atlas-march bodies have no host oracle; mark each leaf | **Fixed.** `emit/mod.rs:529-535`, `:2473-2480`, `:291`; `cf.rs:730-736` re-opened. §4 has a **Kind** column (oracle / oracle over parameters / emit-only) with the rule stated; `hiz_reflect_trace`, `refl_prefilter`/`refl_temporal` and the neighbour loop of `refl_resolve` are emit-only; `sdf_reflect_march` stays oracle'd because `field_distance` is `BufferLoad` and becomes emit-only at the atlas swap. §0 and D16 reworded |
| NB8 | Two citations name `bindless.rs:127-162` and `bindless.rs:1-3` without a crate; in `boyko_render` those lines are `retire_ready_slots` and an unrelated header; both facts live in `boyko_rhi_vulkan` | **Fixed.** `crates/boyko_render/src/bindless.rs:129`, `:161` (retire/exhausted) and `crates/boyko_rhi_vulkan/src/bindless.rs:1-3`, `:139`, `:158` re-opened; R1 and RK-1 now spell the crate, and the sampler citation is `:139-158` (`create_shared_sampler`), not `:127-162` |
| NB9 | R10 "hit shading through the material table = Lumen's hit lighting — no surface cache" never says how a hit is shadowed or where its indirect term comes from; Epic's page describes the cache as the fast path and hit lighting as the costly alternative | **Fixed.** The Lumen page was opened for this revision [D] and quoted; **R10b** specifies direct (CSM/atlas at `P_hit` on mesh legs, `sdf_soft_shadow` on SDF legs, unshadowed past the last cascade — with a second-ray `-D HIT_SHADOW=2` variant), ambient (R1 env at `n_hit` + R9 DDGI at `P_hit`), no punctual lights at the hit in v1 (R10c door), and the variant rows |
| NB10 | RESEARCH §0.2 says `env_brdf_approx` is duplicated in two files; there are six definitions | ~~**Fixed in the RESEARCH** (§0.2 + its C12)~~ — **not done in pass 1; see §15.1. Landed in pass 2**, and the same grep found a THIRD short census (the `R = reflect(-v, n)` hoist, RESEARCH C13) |
| NB11 | Two ladder candidates (reflection-capture normalisation; HDRP sky occlusion) could not be sourced; list them as open | **Fixed here; the RESEARCH half landed in pass 2** (§15.1). §14 here and RESEARCH §7 carry both as UNVERIFIED with the three URLs that did not answer, and where each would sit if real |

### 15.1 Pass 2 (2026-09-10) — what pass 1's own log got wrong

The table above was written as pass 1 closed, and **three of its rows recorded a repair to the
companion document that had not been made**:

- **B1** ends "RESEARCH §0.2 corrected (its C12)". It was not; §0.2 still named two duplicate
  files, and §9 of that document ended at C11 — there was no C12 to cite.
- **NB10** reads "**Fixed in the RESEARCH** (§0.2 + its C12)". Same claim, same absence.
- **NB11** reads "§14 here and RESEARCH §7 carry both as UNVERIFIED". §14 here did; RESEARCH §7
  carried neither candidate.

The design-side half of every one of those rows was real and verified. What failed is narrower and
worse: a critique log — the one artifact a later reader consults *instead of* re-checking — was
itself the thing that went stale, and it went stale in the direction that reads as done. Pass 2
made all three true (RESEARCH §0.2's six-row table, its §7 entry, its new §10 with C12–C16), and
records the miss here rather than quietly satisfying the claim, because "the summary outlives its
retraction" is a failure this repository has catalogued and this is an instance of it.

Two things pass 2 found while landing them, neither of which pass 1 could have found by re-reading
its own text — both needed the grep re-run:

- **A third instance of the B1/NB10 undercount.** RESEARCH §0.2's companion census, "`R =
  reflect(-v, n)` is hoisted once per pixel at" three files, is short by the same three files:
  `sdf_forward_march.comp.hlsl:1035`, `vb_resolve.comp.hlsl:297`, `vb_shade_split.comp.hlsl:451`.
  Nothing in this design depended on it (§2.1 splices where `R` is already in scope, and it is in
  scope at all six), so it is a survey correction (its C13), not a design change — but it is the
  same instrument failing a third time, which is the argument for §2.1's census test rather than
  any list, this one included.
- **NB6 and B4 were seeded in the RESEARCH, not invented here.** §0.4 named the tonemap tail at
  the resolve alone (hence "three producers"), and §0.7 called `gViewT` "the universal
  screen-space input" (hence "exists on every path"). Both are corrected at the source as C14 and
  C15, so a future rung reading the survey does not re-derive the same two errors. This is why
  answering a finding only where the critic pointed is not enough: the critic points at the
  symptom, and the sentence that produced it usually lives one document upstream.

Two smaller repairs, both of the NB8 class (a citation that does not resolve): §14's cross-
reference to "§16 of the RESEARCH" pointed at a section that does not exist (it is §7), and the
two `graph.rs` citations at §2.2 and §14 are now spelled `framegraph/graph.rs`, matching the
`add_image_mipped` citation at §1.4 — `crates/boyko_rhi_vulkan/src/framegraph/graph.rs` is the
only `graph.rs` under this checkout's `crates/`, so this was not ambiguous the way `bindless.rs`
was, only inconsistent with the rule NB8 imposed.

**Standing after pass 2**: 5 blocking + 11 non-blocking findings, **16 fixed, 0 refuted**. Nothing
in either document was found to rest on a finding whose evidence did not survive re-opening — every
`file:line` the critic cited was re-opened and every one held, which is itself worth recording: a
critique pass with a zero refutation rate is one where the reviewer was reading the tree, not the
document.
