// Render PBR MVP-2: the fullscreen deferred Cook-Torrance / GGX RESOLVE.
//
// The marcher (`sdf_gbuffer_composite.hlsl`) writes G-buffer ATTRIBUTES; this pass
// reads them back, fetches the picked material from the material SSBO, runs full
// metallic-roughness Cook-Torrance, and stores the final LIT color. The behavioral
// change vs MVP-1 (which composited `mask ? base*vis : base`) is INTENTIONAL +
// owner-acknowledged (PBR plan call F): SDF (mask == 1) pixels now get full PBR.
// Mesh / background / empty (mask == 0) pixels KEEP the pass-through path BYTE-IDENTICAL
// to MVP-1 (the 0%-gate).
//
//     base   = gAlbedo.rgb;                              // RAW LINEAR base color
//     n      = oct_decode(gNormal.rg);                   // world normal (octahedral)
//     id     = round(gNormal.b*255) | round(gNormal.a*255)<<8;  // 16-bit material id
//     shadow = gMaterial.r;  ao = gMaterial.g;  mask = gMaterial.b > 0.5;
//     m      = materials[id];                            // metallic/roughness/...
//     lit    = mask ? cook_torrance(...) : base;         // STRICT if/select on mask
//
// # The resolve descriptor set (set 0 of the resolve pipeline — NOT the marcher's vocab)
//
//   binding 0 : RWTexture2D<float4> (STORAGE, rgba8) — gAlbedo   (read via `.Load`).
//   binding 1 : RWTexture2D<float4> (STORAGE, rgba8) — gNormal   (oct + material id).
//   binding 2 : RWTexture2D<float4> (STORAGE, rgba8) — gMaterial (shadow, ao, mask).
//   binding 3 : RWTexture2D<float4> (STORAGE, rgba8) — gLit      (the LIT output, store).
//   binding 4 : StructuredBuffer<MaterialGpu>        — the material table (read by id).
//   binding 5 : cbuffer Camera (UNIFORM)             — the 80-byte extent/camera block
//               (the per-pixel view direction is reconstructed from the SHARED ray-gen).
//   binding 6 : StructuredBuffer<uint> (READ-ONLY)   — the Lighting-L0 light table
//               (`[LightHeaderGpu || GpuLight[]]`, word-indexed; see `light_table.hlsli`).
//   binding 7 : RWTexture2D<float> (STORAGE, r32f)   — the Lighting-L0b `gViewT` lane (the
//               marcher's surface ray param `t`), read under `mask == 1` to reconstruct
//               `P = ro + rd * t` for point/spot attenuation.
//
//   binding 8 : StructuredBuffer<uint2> (READ-ONLY)  — the Lighting-L1 ClusterGrid
//               ({offset,count} per froxel; read on the cluster path).
//   binding 9 : StructuredBuffer<uint> (READ-ONLY)   — the Lighting-L1 LightIndexList
//               (the per-froxel light-index slices; the resolve loops the pixel's slice).
//   binding 10: StructuredBuffer<uint> (READ-ONLY)   — the P6 R1 SDF edit-list `Buf` (the
//               per-light `sdf_soft_shadow_ranged` analytic march; decl + contract below).
//   binding 11: RWTexture2D<float> (STORAGE, r8)     — the Render P7 SSAO term `gSsao`,
//               read ONLY when `load_ssao_mode(LightBuf) != 0` (the 0%-gate; decl below).
//
// 12 STORAGE/uniform/buffer bindings (0..=11) — within the resolve binding cap. The G-buffer
// images are consumed in GENERAL (the marcher's STORAGE views, kept in GENERAL after a
// memory-only COMPUTE→COMPUTE barrier) and `gLit` is a storage store. `[[vk::image_format
// ("rgba8")]]` pins each G-buffer `OpTypeImage` to `Rgba8` (shaderStorageImageWriteWithoutFormat
// is OFF); `gViewT` is pinned `r32f` and `gSsao` `r8`.
//
// # BRDF (the Filament/Karis real-time convergence, PBR P0 — multi-scatter compensated)
//
//   D = GGX/Trowbridge-Reitz; V = height-correlated Smith visibility (folds 1/(4 NoL NoV));
//   F = Schlick; diffuse = Lambert (albedo/PI). Metallic-roughness:
//     f0 = lerp(0.16*reflectance^2, base, metallic);  diffuse = base*(1-metallic).
//   Direct light: one analytic directional light, modulated by the A1 shadow.
//   Ambient/IBL: analytic EnvBRDFApprox (Karis mobile) specular sampled along the
//   REFLECTION vector against a STEEPENED sky/ground gradient (PBR P0-B — a metal mirrors
//   its surroundings, not a flat tint; the smoothstep gives it a real bright-cap/dark-belly
//   split instead of a flat mid-tone) + a hemisphere diffuse ambient sampled along N. AO is
//   DECOUPLED (the PBR metal fix): diffuse ambient is modulated by the A2 AO (`ao_final`),
//   ambient SPECULAR by a separate roughness-aware `spec_ao` (Filament SpecularAO_Lagarde,
//   ~1 for smooth/metal) — a metal's diffuse is 0, so its ambient specular IS its entire
//   appearance and must not be AO-darkened like matte paint. A 1-mul multi-scatter
//   energy-compensation term (PBR P0-D) recovers the single-scatter GGX energy loss at high
//   roughness, applied at every specular site (direct + ambient). No IBL texture, no LUT (MVP-2).
//   OUTPUT: the accumulated linear radiance is exposure-scaled, then tonemapped by the
//   SELECTED operator (`tonemap_select`, word-7 bits 8..11 — DEFAULT Hill ACES-fitted,
//   hue-preserving highlight rolloff, PBR P0-C; Khronos PBR Neutral / Reinhard-Jodie are
//   selectable via `LightingConfig::tonemapper`), then manually gamma-2.2 encoded (the
//   gLit/swapchain chain is linear UNORM end to end — no hardware sRGB). The host oracle
//   (`golden_deferred_resolve*` in goldens.rs) models this identically.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 deferred_pbr.hlsl \
//       -Fo deferred_pbr.comp.spv
//   (TERMINATOR_WRAP variant, SOFTWARE-RESOLVE-ONLY: add `-D TERMINATOR_WRAP=1
//       -Fo deferred_pbr_wrap.comp.spv` — the diffuse terminator light-wrap at both
//       direct-light accumulation sites, frozen-base discipline: with the flag UNDEFINED
//       this source preprocesses CHARACTER-IDENTICAL to the pre-feature file, so the base
//       `.spv` above is untouched by construction. An HWRT + TERMINATOR_WRAP combo is
//       explicitly OUT OF SCOPE this rung — never compiled, never selected by the host.)
//   (HWRT variant, feature="hwrt": add `-T cs_6_5 -D HWRT=1
//       -Fo deferred_pbr_hwrt.comp.spv` — RESOLVE_INLINE, the default SHADOW_STAGE; see
//       compute.rs's `DEFERRED_PBR_HWRT_SPV` embed doc.)
//   (HWRT VIS variant: add `-T cs_6_5 -D HWRT=1 -D SHADOW_STAGE=1
//       -Fo deferred_pbr_hwrt_vis.comp.spv` — writes gShadowVis and returns before
//       lighting, the à-trous spatial-denoise pre-pass; see compute.rs's
//       `DEFERRED_PBR_VIS_SPV` embed doc.)
//   (HWRT DENOISED variant: add `-T cs_6_5 -D HWRT=1 -D SHADOW_STAGE=2
//       -Fo deferred_pbr_hwrt_denoised.comp.spv` — reads the à-trous output instead of
//       tracing; see compute.rs's `DEFERRED_PBR_DENOISED_SPV` embed doc.)
//   (HWRT VIS+MOTION_VECTORS variant: add `-T cs_6_5 -D HWRT=1 -D SHADOW_STAGE=1
//       -D MOTION_VECTORS=1 -Fo deferred_pbr_hwrt_vis_mv.comp.spv` — VIS plus the
//       per-pixel camera-only motion vector the temporal shadow denoiser reprojects with;
//       see compute.rs's `DEFERRED_PBR_VIS_MV_SPV` embed doc.)

static const float PI = 3.14159265358979323846;

// bindings 0..3: the G-buffer + lit storage images. `[[vk::image_format("rgba8")]]` pins
// each `OpTypeImage` to `Rgba8` to match the R8G8B8A8_UNORM views.
[[vk::image_format("rgba8")]] RWTexture2D<float4> gAlbedo   : register(u0);
[[vk::image_format("rgba8")]] RWTexture2D<float4> gNormal   : register(u1);
[[vk::image_format("rgba8")]] RWTexture2D<float4> gMaterial : register(u2);
[[vk::image_format("rgba8")]] RWTexture2D<float4> gLit      : register(u3);

// binding 4: the material table (std430 MaterialGpu; mirrors boyko_render::MaterialGpu).
//   off 0  : float4 base_color   rgb = linear base color, w = alpha/cutoff
//   off 16 : float4 mrr          [metallic, roughness, reflectance, bitcast(flags)]
//   off 32 : float4 emissive     rgb = linear emissive, w unused
struct MaterialGpu {
    float4 base_color;
    float4 mrr;
    float4 emissive;
};
StructuredBuffer<MaterialGpu> Materials : register(t4);

// binding 5: the camera/extent UNIFORM block (byte-identical to the marcher's `Camera` +
// the host `CompositePushConstants`). The resolve uses it for the extent (1:1 the marched
// pixels) and the per-pixel view direction (the shared ray-gen).
cbuffer Camera : register(b5) {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};

// binding 6: the Lighting-L0 light table (word-indexed `[LightHeaderGpu || GpuLight[]]`;
// std430 layout decoded by `light_table.hlsli`, mirrored by boyko_render::light + the
// host oracle). Replaces the compiled-in LIGHT_DIR/LIGHT_COLOR/SKY_* constants below.
StructuredBuffer<uint> LightBuf : register(t6);

// binding 7: the Lighting-L0b `gViewT` G-buffer lane (R32_SFLOAT STORAGE image in GENERAL,
// the marcher's surface ray param `t`). READ ONLY inside the `is_sdf_lit` / `mask == 1`
// branch (C2 read-under-mask gate) to reconstruct the world position `P = ro + rd * t` for
// point/spot attenuation — a `1.0e30` sentinel on a non-lit pixel is therefore never
// consumed. `[[vk::image_format("r32f")]]` pins the `OpTypeImage` to `R32f` (matching the
// marcher's store view). (The L0a light_table occupies binding 6, so `gViewT` lands at
// binding 7 — both ≤ the 12-binding cap.)
[[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(u7);

// binding 8 / 9: the Lighting-L1 cluster grid + flat light-index list (read-only here; the
// `cluster_cull.hlsl` pass writes them). When `clusters_enabled` (header `cluster_params.w`),
// the resolve maps the pixel to its froxel, reads `ClusterGrid[cluster].{offset,count}`, and
// loops ONLY `LightIndexList[offset .. offset+count)` for the point/spot block — instead of
// the brute-force `[l0a_count .. light_count)` flat loop (the L0b path, kept as the L1 OFF /
// 0%-gate). The cluster index linearization + the exp-Z slice math are the shared
// `light_table.hlsli` helpers, byte-identical to the cull write. Both ≤ the 12-binding cap.
StructuredBuffer<uint2> ClusterGrid : register(t8);
StructuredBuffer<uint> LightIndexList : register(t9);

// binding 10 (P6 R1): the SDF edit-list SSBO (the SAME `Buf` the marcher binds + uploads +
// barriers; the resolve dispatch is ordered after the marcher in the same submit, so the
// prior upload+barrier already covers this second COMPUTE read — no new barrier). The
// `sdf_field.hlsli` INCLUDE CONTRACT requires `StructuredBuffer<uint> Buf : register(t0)` in
// scope BEFORE the include; the resolve's `t0` SRV register is free (it uses t4/t6/t8/t9),
// and Vulkan binding 10 is free under the 12-binding cap (10 → 11 bindings; NO cap raise —
// the orchestrator's R1=(A) analytic-march decision drops the brick-atlas binds). The
// resolve is a strict FIELD-CONSUMER: it CALLS `field_distance` read-only, never edits.
[[vk::binding(10)]] StructuredBuffer<uint> Buf : register(t0);

// binding 11 (Render P7): the SSAO term — a full-res `R8_UNORM` STORAGE image carrying the
// per-pixel HBAO-lite ambient occlusion the (C2) SSAO pass writes. READ ONLY inside the
// `is_sdf_lit` ambient combine when `load_ssao_mode(LightBuf) != 0u` (the structural 0%-gate):
// on a `ssao_mode == 0` scene (every pre-P7 scene) `gSsao.Load` is never executed and the
// binding is a harmless valid descriptor, so the resolve is arithmetically byte-identical to
// today. `[[vk::image_format("r8")]]` pins the `OpTypeImage` to `R8` (matching the R8_UNORM
// view; the SSAO pass / placeholder both bind an R8 image). The descriptor is present on
// EVERY resolve layout (the interface is stable regardless of DXC dead-code elimination).
[[vk::image_format("r8")]] RWTexture2D<float> gSsao : register(u11);

// === CSM Increment 1b — Rung A: the cascade shadow map + comparison sampler + cascade UBO =====
//
// bindings 12/13 (the resolve set grows 12 → 14 bindings; the 16-binding cap leaves 2 free — see
// the W4 `debug_assert` on the layout build). Both are BOUND-BUT-UNREAD on the OFF path
// (`load_csm_mode(LightBuf) == 0`, every pre-CSM scene): the resolve `.spv` STATICALLY references
// them, so the layout MUST declare + a valid descriptor MUST be bound (a 1×1×1 D32 array dummy +
// the comparison sampler as ONE combined descriptor + a zeroed cascade UBO), but the
// `SampleCmpLevelZero` only executes inside the `csm_mode != 0` structural `if`, so on the OFF path
// the dummies are never sampled → the lit PIXELS are byte-identical to today (the gSsao precedent;
// the 0%-gate).
//
// binding 12 (t12 + s12): the cascade shadow-map ARRAY (Rung A: 1 layer) BUNDLED with its PCF
// COMPARISON sampler as ONE `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER`. `gCsm` (t12) +
// `gCsmCmp` (s12) share the SAME register NUMBER, so DXC collapses them into one combined
// descriptor at binding 12 — the EXACT precedent the marcher's `BrickAtlas`(t10)+`BrickSampler`(s10)
// uses. DEVIATION from the Inc-1b plan's "binding 13 = a separate comparison-sampler descriptor":
// the in-house RHI's `BindGroupEntry` vocabulary has no SAMPLER-only variant (only
// `CombinedImage`), and adding one is a cross-cutting RHI change beyond this task's edit surface;
// the combined descriptor is functionally identical (a PCF `SampleCmpLevelZero` over the array
// layer) and keeps the resolve set within the cap. `gCsm` is `Texture2DArray<float>` (the depth
// pass renders cascade `c` into layer `c`; the resolve PCF-samples `float3(uv, c)`; Rung A uses
// ONLY `c == 0`); the sampler is `compareEnable = VK_TRUE` / `LessOrEqual` (Inc-0
// `SamplerDesc.compare = Some(LessOrEqual)`).
Texture2DArray<float> gCsm : register(t12);
SamplerComparisonState gCsmCmp : register(s12);

// One cascade's GPU-ready record — MUST byte-mirror `boyko_render::CascadeData` (80 B): the
// COLUMN-MAJOR world→light-clip `view_proj` (O1: SAME majorness as the depth VS push — DXC default,
// NO `row_major`) + the VIEW-space `split_far` + the world-space `texel_size` + 8 B pad to the
// 16-byte cbuffer-array stride. The HLSL `float4x4` is 64 B (4 × 16) and the trailing 3 scalars +
// pad fill one final 16-B row → 80 B, identical to the `#[repr(C)]` host struct.
struct CascadeData {
    float4x4 view_proj;   // column-major world→light-clip (O1 majorness pin)
    float    split_far;   // VIEW-space far distance of this cascade (Rung B selection boundary)
    float    texel_size;  // world-space size of one shadow texel (the normal-bias scale)
    float2   _pad;        // pad to the 16-byte cbuffer-array stride
};

// binding 13 (b13): the cascade UBO — byte-mirrors `boyko_render::ResolvedCsm` (336 B): the inline
// `CascadeData[MAX_CASCADES]` (4 × 80 = 320 B) + `active_count` + `csm_mode_word` +
// `pcf_kernel_word` + 4 B pad. The host uploads `ResolvedCsm` verbatim each frame. `gCsmMode`
// mirrors `csm_mode_word` (a redundant copy of the header bit, carried for completeness); the
// resolve gates on the HEADER's `load_csm_mode` (the single source of truth), NOT this field.
// `gCsmPcfKernel` (rung E1) mirrors `pcf_kernel_word` — `csm_pcf_disc` (`shadow_apply.hlsli`)
// branches on it directly.
static const uint MAX_CASCADES = 4u;
cbuffer CsmCascades : register(b13) {
    CascadeData gCascades[MAX_CASCADES];
    uint gCsmActive;      // number of valid cascades (0 = disabled); mirrors ResolvedCsm.active_count
    uint gCsmMode;        // mirrors ResolvedCsm.csm_mode_word (the resolve gates on the header bit)
    uint gCsmPcfKernel;   // mirrors ResolvedCsm.pcf_kernel_word (rung E1: the CsmPcfKernel word)
    uint _gCsmPad;        // pad to the 336-byte ResolvedCsm stride
};

// Multi-paradigm render-path plan, rung R4b (Decision 3/7 extraction): `CSM_NORMAL_BIAS` moved
// to `shadow_apply.hlsli` (it is read only by `csm_sample_cascade`, which moved with it) —
// `#include`d below, after the DDGI resolve include.

// === Shadow Phase 5 Inc-1-GPU — the sparse SPOT/POINT atlas (binding 14 + 15) ==================
//
// binding 14 (t14/s14): the shadow-atlas array map + its PCF COMPARISON sampler as ONE combined
// `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER` (the `gCsm` precedent). `gShadowAtlas` (t14) +
// `gShadowAtlasCmp` (s14) share the SAME register NUMBER, so DXC collapses them into one combined
// descriptor — keeping the resolve set at 16/16 (the cap). `gShadowAtlas` is
// `Texture2DArray<float>` (the depth pass renders spot slot `s` into layer `s`; the resolve
// PCF-samples `float3(uv, s)`); the sampler is `compareEnable = VK_TRUE` / `LessOrEqual`. ALWAYS
// bound (the resolve `.spv` STATICALLY references it); the `SampleCmpLevelZero` only executes inside
// the `punctual_shadow_mode != 0` structural `if`, so on the OFF path the bound-but-unread dummy is
// never sampled (the 0%-gate; the `gCsm`/`gSsao` precedent).
Texture2DArray<float> gShadowAtlas : register(t14);
SamplerComparisonState gShadowAtlasCmp : register(s14);

// The shadow-atlas slot budget — MUST equal `boyko_render::shadow_atlas::M_SLOTS` (16) and the
// depth target's `array_layers`. Bounds the `gFaces` array + the `light_atlas_slot` range.
static const uint M_SLOTS = 16u;

// One atlas layer's GPU-ready transform — MUST byte-mirror `boyko_render::FaceTransform` (80 B): the
// COLUMN-MAJOR world→light-clip `view_proj` (O1: SAME majorness as the depth-pass push — DXC
// default, NO `row_major`) + the POINT-shared `light_pos` (Inc-2 cube distance-compare; unused by
// the SPOT NDC-z compare) + `inv_range` + 8 B pad to the 16-byte cbuffer-array stride. Identical
// shape + stride to `CascadeData`, so the shared shadow upload path treats a cascade and an atlas
// face identically.
struct FaceTransform {
    float4x4 view_proj;   // column-major world→light-clip (O1 majorness pin)
    float3   light_pos;   // world light position (Inc-2 POINT cube; unused by SPOT NDC-z)
    float    inv_range;   // reciprocal range (Inc-2 POINT normalized-distance; unused by SPOT)
};

// binding 15 (b15): the shadow-atlas UBO — byte-mirrors `boyko_render::ResolvedShadowAtlas` (1296
// B): the inline `FaceTransform[M_SLOTS]` (16 × 80 = 1280 B) + `active_layers` + `mode_word` + 8 B
// pad. The host uploads `ResolvedShadowAtlas` verbatim each frame. `gAtlasMode` mirrors `mode_word`
// (a redundant copy of the header bit, carried for completeness); the resolve gates on the HEADER's
// `load_punctual_shadow_mode` (the single source of truth), NOT this field.
cbuffer ShadowAtlas : register(b15) {
    FaceTransform gFaces[M_SLOTS];
    uint gAtlasActive; // number of valid atlas layers (mirrors ResolvedShadowAtlas.active_layers)
    uint gAtlasMode;   // mirrors ResolvedShadowAtlas.mode_word (the resolve gates on the header bit)
    uint2 _gAtlasPad;  // pad to the 1296-byte ResolvedShadowAtlas stride
};

// === SDFDDGI I0 — the DDGI probe-irradiance + depth atlases + grid UBO (bindings 16/17/18) ======
//
// The octahedral probe grid (Hu et al. 2021 / RTXGI DDGI) whose irradiance the resolve samples into
// the `ambient` accumulator. Declared NOW (I0 — the gated skeleton) but READ by NOTHING: the GI
// injection block below is EMPTY at I0 (I3 wires the trilinear probe sample). All three are
// BOUND-BUT-UNREAD on the OFF path (`load_ddgi_mode(LightBuf) == 0`, every pre-SDFDDGI scene) — the
// resolve `.spv` STATICALLY declares them so the layout MUST bind valid descriptors, but no
// `.Sample` executes, so the lit PIXELS are byte-identical to today (the `gCsm`/`gShadowAtlas`
// bound-but-unread precedent, the 0%-gate). The 3 extra bindings sit at 16/17/18 → the SOFTWARE
// resolve set is exactly 19/19 (the HWRT variant adds TLAS @19 + the shadow-params UBO @20 under the
// rung-1b `MAX_BIND_GROUP_BINDINGS == 21` cap; the software fill is unchanged).
//
// binding 16 (t16 + s16): the probe IRRADIANCE atlas (R11G11B10F, no-gamma — Decision D6)
// Texture2DArray BUNDLED with its LINEAR sampler as ONE `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER`
// (the `gCsm`(t12+s12) / `gShadowAtlas`(t14+s14) combined-image-collapse precedent — DXC shares the
// register NUMBER). `gDdgiIrr` is `Texture2DArray<float4>` (octahedral irradiance tiles per plane);
// I0 binds a bound-but-unread dummy array.
Texture2DArray<float4> gDdgiIrr : register(t16);
SamplerState gDdgiIrrSamp : register(s16);

// binding 17 (t17 + s17): the probe DEPTH-MOMENT atlas (RG16F: mean + mean²) Texture2DArray BUNDLED
// with its LINEAR sampler as ONE combined descriptor (same collapse). `gDdgiDepth` is
// `Texture2DArray<float2>` (the two-moment Chebyshev leak-suppression tiles); I0 binds a
// bound-but-unread dummy array.
Texture2DArray<float2> gDdgiDepth : register(t17);
SamplerState gDdgiDepthSamp : register(s17);

// binding 18 (b18): the DDGI grid UBO — byte-mirrors `boyko_render::ResolvedDdgi` (48 B): the grid
// `origin` (vec4) + `inv_spacing` and the three `u32` dims (packed in one vec4) + `ddgi_mode_word` +
// pad. The host uploads `ResolvedDdgi` verbatim; the grid is WORLD-FIXED (Decision D1), so this UBO
// needs NO per-FIF ring. `gDdgiMode` mirrors `ddgi_mode_word` (a redundant copy of the header bit);
// the resolve gates on the HEADER's `load_ddgi_mode` (the single source of truth), NOT this field.
// UNREAD at I0 (the injection block is empty).
cbuffer ResolvedDdgi : register(b18) {
    float4 gDdgiOrigin;     // grid origin (probe (0,0,0) min world corner); .w padding
    float4 gDdgiInvSpacDims;// .x = inv_spacing; .yzw = bit-cast u32 dims (x, y, z)
    uint   gDdgiMode;       // mirrors ResolvedDdgi.ddgi_mode_word (the resolve gates on the header bit)
    uint3  _gDdgiPad;       // pad to the 48-byte ResolvedDdgi stride
};

#if !HWRT
// Textured-PBR T6a (binding 19, SOFTWARE-RESOLVE-ONLY; the critic's C1 fix): the `gPbr`
// deferred-resolve MRT lane the (T6c) textured raster writes — r=metallic, g=roughness,
// b=AO-texture modulation, a=emissive-strength modulation. Declared ENTIRELY under `#if
// !HWRT` so no HWRT `.spv` ever references it (the byte-identity gate — mirrors the TLAS's
// `#if HWRT` symmetry at the SAME binding index; the two arms are mutually exclusive
// compiles of this ONE source file, never both present in the same `.spv`). The `.Load`
// below is INSIDE the flag-gated branch, so it never executes for a flag=0 material (every
// current material) — the discarded/unwritten contents are never observed.
[[vk::binding(19)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> gPbr;

// A bit in `MaterialGpu.mrr.w`'s bitcast `flags` lane, set iff the material carries a
// texture sidecar (mirrors `boyko_render::MATERIAL_FLAG_TEXTURED`). The Rust-side copies
// of this bit (the `MATERIAL_FLAG_TEXTURED` const and its two callers,
// `MaterialTable::seed_rows`'s derive-at-upload and `Material::with_textures`'s CPU-side
// mirror) are cross-checked against the shader's `.spv` output by the runtime test
// `textured_pbr_t6a_host_shader_agreement.rs`; this HLSL literal agrees with them by
// source-comment convention (there is no machine-checked host↔shader assert on the value
// `1u` itself — unlike e.g. `MATERIAL_GPU_WORDS`, which IS const-asserted host-side).
static const uint MATERIAL_FLAG_TEXTURED_BIT = 1u;
#endif

#if HWRT
// binding 19 (t19): the per-frame TLAS (R2a-3 `PersistentTlas.accel`) the HWRT mesh-shadow variant
// traces with `rayQuery` (R2a-4b). Declared ENTIRELY under `#if HWRT` so the software `.spv` never
// references an acceleration structure (the byte-identity gate). `RaytracingAccelerationStructure`
// binds as `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR` at set 0 binding 19 — the 20th resolve
// descriptor the HWRT layout adds atop the 19 the software resolve uses.
[[vk::binding(19)]] RaytracingAccelerationStructure tlas;

// R2a-4b HWRT mesh-shadow ray tuning (world space — the directional light is at infinity, so the ray
// is a world-space cast; a VIEW-space `split_far` is dimensionally wrong here, critic P0-2). The
// origin is lifted off the surface by `n * SHADOW_RAY_BIAS` and the trace runs [`SHADOW_RAY_TMIN`,
// `SHADOW_RAY_TMAX`]. `TMAX = 1e4` covers the bounded scene; the MAGNITUDE (bias / TMin) is tuned at
// owner-eval, the DIMENSION is fixed world-space.
//
// Rung 1b — tunable soft-shadow params:
//   * SHADOW_RAY_COUNT is spec-const id 0 (baked at pipeline build; the Vogel loop below UNROLLS
//     against the baked count, so a runtime change is a relaunch — Decision 5). The `16` here is the
//     default when unspecialized (byte-identical to the R2a-4b hardcoded const).
//   * cone/tmax/tmin/bias come from RayShadowUbo @ binding 20 — a per-FIF UBO byte-mirroring
//     `boyko_render::ResolvedRayShadow` (4×f32: cone_radius, tmax, tmin, bias). Runtime-tunable,
//     defaults byte-identical to the old consts.
//   * SHADOW_FRAME_SEED (rung 3b temporal) is the runner's per-frame counter, packed straight
//     into the UBO at `upload_ray_shadow_ring` (a HOT per-frame value, NOT routed through the
//     cold `ResolvedRayShadow` resolve — see that struct's doc). It advances the Vogel-disk cone
//     rotation below by the golden angle each frame, so the temporal shadow denoiser has
//     something to average (a frame-invariant rotation is measured dead weight for it).
// binding 20 (b20): the tunable soft-shadow params UBO. Field ORDER + TYPES exactly match
// `boyko_render::ResolvedRayShadow` (cone_radius @0, tmax @4, tmin @8, bias @12) PLUS the
// runner-injected SHADOW_FRAME_SEED @16 — 20 B used of a 32 B std140 block (rounded to the next
// vec4 boundary; the trailing 12 B is bound-but-unread pad). Declared ENTIRELY under `#if HWRT`
// — the software `.spv` never references it (the byte-identity gate).
cbuffer RayShadowUbo : register(b20) {
    float SHADOW_CONE_RADIUS; // was 0.035 (tan(half-angle) of the sun disk, ~2°)
    float SHADOW_RAY_TMAX;    // was 1e4
    float SHADOW_RAY_TMIN;    // was 1e-3
    float SHADOW_RAY_BIAS;    // was 1e-3
    uint  SHADOW_FRAME_SEED;  // rung 3b: per-frame counter, offset 16 (see above)
};

// R2a-4b soft-shadow (owner-eval): the hard single-ray trace read TOO SHARP, so the mesh-shadow
// term cone-samples N rays jittered within the sun's angular disk around `l` and averages the miss
// fraction — a single-frame soft penumbra (no TAA on this engine, so the per-ray count carries the
// smoothness). `SHADOW_CONE_RADIUS` is `tan(half-angle)` of the sun disk (~2°); rung 1b moved it +
// tmax/tmin/bias into the RayShadowUbo above and SHADOW_RAY_COUNT to spec-const id 0.
[[vk::constant_id(0)]] const uint SHADOW_RAY_COUNT = 16; // rays per pixel (spec-const; default 16)
#endif

// SHADOW_STAGE selects how the HWRT mesh-shadow term is produced (Rung 3a spatial denoise):
//   RESOLVE_INLINE  (default) - trace inline + light, exactly as before (byte-identical).
//   VIS             - trace, write raw visibility to gShadowVis, return before lighting.
//   RESOLVE_DENOISED- read the a-trous-filtered visibility instead of tracing.
#define SHADOW_STAGE_RESOLVE_INLINE   0
#define SHADOW_STAGE_VIS              1
#define SHADOW_STAGE_RESOLVE_DENOISED 2
#ifndef SHADOW_STAGE
#define SHADOW_STAGE SHADOW_STAGE_RESOLVE_INLINE
#endif

#if SHADOW_STAGE != SHADOW_STAGE_RESOLVE_INLINE
// binding 21 (u21): the shadow-visibility image (Rung 3a spatial denoise). RG: R = raw mesh_vis,
// G = validity (1 = a real mesh-shadow sample was written; 0 = the neutral fill on a pixel that
// never reached the mesh arm). Declared ENTIRELY under `SHADOW_STAGE != RESOLVE_INLINE` so the
// RESOLVE_INLINE `.spv` (the byte-identity gate) never references it — 21 is the next free HWRT
// binding after the TLAS @19 + RayShadowUbo @20. ONE binding serves BOTH stages: the VIS stage
// UAV-WRITES `float2(mesh_vis, 1.0)` here, and the RESOLVE_DENOISED stage `.Load`s the à-trous-
// FILTERED value the host binds into this same slot (the host swaps the descriptor between stages).
// `[[vk::image_format("rg16")]]` pins the `OpTypeImage` to `Rg16` (`shaderStorageImageWriteWithout-
// Format` is OFF). BOTH ping-pong rings (`shadow_vis` + `shadow_vis2`) AND this binding are the
// SAME format, R16G16_UNORM (uniform-RG16 design): the VIS stage writes `shadow_vis[fi]` (RG16) and
// the RESOLVE_DENOISED stage reads the FINAL à-trous output (also RG16, either ring by parity), so
// the single "rg16" pin matches the bound view on EVERY level and every `levels` value — no
// format-class mismatch on the odd-parity or DENOISED bind (the former RG8-vs-RG16 divergence).
[[vk::image_format("rg16")]] RWTexture2D<float2> gShadowVis : register(u21);
#endif

#ifdef MOTION_VECTORS
// Rung 3b step 5b: the SDF-pixel motion-vector output + the camera pair. Declared ONLY under
// MOTION_VECTORS (the `deferred_pbr_hwrt_vis_mv` variant — SHADOW_STAGE=VIS + MOTION_VECTORS), so
// the base VIS / DENOISED / RESOLVE_INLINE `.spv` never reference bindings 22/23 and their layouts
// stay the frozen byte-identity gate. binding 22: the `MotionCam` UBO (current + previous
// marcher-aligned proj*view, column-major — the SAME 128 B camera pair the raster MV variant reads,
// so the mesh and SDF motion vectors share ONE camera basis). binding 23 (u23): the `motion_vec`
// image the raster wrote MESH pixels into; this stage adds the SDF pixels (camera-only). Pinned
// **rg16f** (`R16G16_SFLOAT`, matching the image) — NOT `rg16`/UNORM: Δuv is SIGNED and can exceed
// [0,1], so a UNORM pin would clamp negative/>1 motion and disagree with the raster's SFLOAT pixels.
[[vk::binding(22)]] cbuffer MotionCamVis {
    float4x4 mv_cur_view_proj;   // current marcher-aligned proj*view
    float4x4 mv_prev_view_proj;  // last frame's marcher-aligned proj*view
};
[[vk::image_format("rg16f")]] RWTexture2D<float2> gMotionVec : register(u23);

// Marcher-aligned clip -> [0,1]^2 screen UV. The projection (`marcher_view_proj_rows`) bakes the
// y-flip into clip.y, so this is the plain NDC remap (NO extra negation) — IDENTICAL to the gbuffer
// MV variant's `clip_to_uv`, so the mesh (raster) and SDF (here) motion vectors land in ONE
// consistent UV space across the r1 ownership seam.
float2 mv_clip_to_uv(float4 clip) {
    return (clip.xy / clip.w) * 0.5 + 0.5;
}
#endif

// Multi-paradigm render-path plan, rung R4b (Decision 3/7 extraction): `SPOT_SHADOW_NORMAL_BIAS`
// and `CSM_OVERLAP_PROPORTION` moved to `shadow_apply.hlsli` (read only by
// `spot_atlas_visibility`/`punctual_atlas_visibility` and `csm_visibility`, which moved with
// them) — `#include`d below, after the DDGI resolve include.

// Shared camera ray-gen (the SAME header the marcher includes — ONE ray-gen, no drift).
#include "ray_gen.hlsli"
// Shared light-table std430 decode (ONE source of truth, included by the resolve + cull).
#include "light_table.hlsli"
// P6 R1: the FROZEN shared SDF field gateway (`field_distance`) — for the per-light analytic
// `sdf_soft_shadow_ranged` march. Included AFTER `Buf` (the include contract). A strict
// field-CONSUMER; the field math + `sdf_field.hlsli` stay BYTE-FROZEN.
#include "sdf_field.hlsli"
// SDFDDGI I3: the shared DDGI resolve sample (`ddgi_probe_sample`) — ONE source of truth with the
// `ddgi_probe_gi_resolve` GPU golden, the op-for-op HLSL mirror of `goldens::probe_sample`.
// Included AFTER the gDdgiIrr/gDdgiDepth/ResolvedDdgi binding decls above (the tap helpers read
// them). GI-OFF (`ddgi_mode == 0`) never calls into it — the 0%-gate holds.
#include "ddgi_resolve.hlsli"
// Multi-paradigm render-path plan, rung R4b (Decision 7 / W2): the combined shadow-source apply
// — `csm_visibility`/`spot_atlas_visibility`/`punctual_atlas_visibility` + their helpers, a
// VERBATIM textual cut (see that header's doc). Included AFTER the CSM/atlas binding decls above
// (its INCLUDE CONTRACT precondition) — the same "declare, then include" idiom `sdf_field.hlsli`'s
// `Buf` precondition and `pbr_lighting.hlsli`'s `PI`/`LIGHT_UP` precondition use.
#include "shadow_apply.hlsli"

// P6 R1 shadow-march tuning — MIRRORS the marcher's frozen A1 consts (`sdf_gbuffer_
// composite.hlsl:407-437`) byte-for-byte (the same owner defaults; `GRAD_H` +
// `FIELD_LIPSCHITZ_L` come from `sdf_field.hlsli`). The `sdf_soft_shadow_ranged` body spells
// these symbolically; they are value-identical to the marcher's so the ranged march matches
// the marcher's analytic shadow up to the per-caster `t_max` bound. `T_MAX` is the extra-
// directional caster's march bound (a punctual caster passes the light DISTANCE instead).
static const float EPS              = 0.001;
static const float T_MAX            = 10.0;
static const uint  MAX_IT           = 128u;
static const float SHADOW_K         = 8.0;
static const float SHADOW_MINT      = 16.0 * GRAD_H;
static const float SHADOW_MINT_STEP = 16.0 * GRAD_H;
static const float SHADOW_HIT_EPS   = 2.0 * EPS;
static const float SHADOW_NDOTL_EPS = 0.0;
static const float SHADOW_NORMAL_BIAS = 0.02; // normal-offset march-origin lift (anti grazing-acne)

// P6 R1 cap: the maximum number of EXTRA shadow casters marched per pixel (the dominant-N
// bound, Decision 2/7). Beyond this, flagged lights contribute NoL-only (no march). Mirrors
// the host `MAX_SDF_SHADOW_CASTERS_PER_PIXEL`. Owner-retunable.
static const uint MAX_SDF_SHADOW_CASTERS_PER_PIXEL = 4u;

// === Render Shadow Phase 3 — Screen-Space Contact Shadows (SSCS) tuning =====================
//
// A short ray-march in SCREEN SPACE along the light direction `l`, sampling the depth G-buffer
// (`gViewT`) to detect a near occluder the SDF/analytic shadow misses (fine contact gaps where
// a foot meets the floor, etc.). Multiplied INTO the per-light `vis` factor at both lighting
// sites, gated by `contact_shadow_mode` (header word 7 bit 1; OFF on every pre-Phase-3 scene →
// the structural-`if` block never runs → byte-identical to today). Hand-written, owner-retunable.
static const uint  SSCS_STEPS           = 8u;    // march sample count along `l`
static const float SSCS_CONTACT_LENGTH  = 0.25;  // world-space march length (the contact reach)
static const float SSCS_THICKNESS_FLOOR = 0.07;  // min occluder-thickness tolerance (anti light-leak)
static const float SSCS_EDGE_FADE_K     = 6.0;   // HDRP screen-edge vignette steepness
static const float SSCS_DISTANCE_FADE   = 50.0;  // disable SSCS past this view depth (far surfaces)

// Render P7 POLISH follow-up: the SSAO denoise MOVED OUT of this resolve into a dedicated
// multi-pass edge-avoiding à-trous compute chain (`ssao_atrous.comp.hlsl`, mirroring the
// SHIPPED `shadow_atrous.comp.hlsl` RT soft-shadow denoiser) — see that file's header doc for
// the filter itself (the Dammertz 5-tap B3-spline, the plane-fit depth gate, linear-Z). The
// former inline 15x15 bilateral blur (Render P7 POLISH Change C) is RETIRED from this file: the
// combine below now `.Load`s the à-trous chain's FINAL filtered lane directly (a single
// `gSsao.Load`, no per-pixel neighbourhood walk in the resolve). `SSAO_BLUR_R` /
// `SSAO_BLUR_SPATIAL_SIGMA` / `SSAO_BLUR_DEPTH_TOL` / `SSAO_BLUR_DEPTH_SIGMA` /
// `SSAO_BLUR_GRAD_CLAMP` moved to `ssao_atrous.comp.hlsl` (the depth gate lives entirely inside
// the à-trous pass now); this file keeps only the bare `1.0e30` background sentinel the combine
// below still reads.

// VB-SV0 (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §4.1): `sdf_soft_shadow_ranged` — which lived
// VERBATIM at THIS point in this file — moved into the shared `sdf_shadow_leaves.hlsli` so the
// three VB lit-producer tails can consume the SAME definition instead of hand-copying it. The
// `#include` sits at exactly the point the moved span occupied, so this file's token stream is
// unchanged where it matters and all SIX `deferred_pbr` `.spv` stay byte-identical
// (`cluster_grid_read_bound.rs::deferred_and_forward_families_spv_byte_identical` is that gate).
// The header ALSO carries `sdf_ao` + the three A2 AO consts, which this file never referenced
// and still does not — they arrive as unreferenced declarations that DXC strips, which is
// asserted by the same byte gate rather than assumed.
//
// Its INCLUDE CONTRACT is already satisfied here: `field_distance` (`sdf_field.hlsli`, included at
// :447 — BELOW `Buf`'s decl at :161, which is the order `sdf_field.hlsli`'s own contract requires)
// plus the `MAX_IT`/`SHADOW_K`/`SHADOW_MINT`/`SHADOW_MINT_STEP`/`SHADOW_HIT_EPS` block declared
// immediately above at :466-474, and `FIELD_LIPSCHITZ_L` from `sdf_field.hlsli`.
#include "sdf_shadow_leaves.hlsli"

// The legacy 64x64 fixture extent when the UBO extent is zero (mirrors the marcher).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// --- Lighting (Lighting L0a) -----------------------------------------------------------
//
// The compiled-in MVP-2 LIGHT_DIR / LIGHT_COLOR / SKY_DIFFUSE / SKY_SPEC constants are
// REPLACED by the L0a light table: the resolve loops the header's no-`P` front block
// (`[0..l0a_count)`) handling `kind == Directional` (the Cook-Torrance direct path) and
// `kind == Sky` (the hemisphere ambient). The 0%-gate degenerate table — one directional
// (dir = +Z, white, illuminance 1.0) + one sky (`sky == ground == (0.10,0.10,0.12)`),
// exposure 1.0 — reproduces the old constants byte-for-byte. The world up the sky lerp
// interpolates against.
static const float3 LIGHT_UP = float3(0.0, 1.0, 0.0);

// --- Octahedral decode (the inverse of the marcher's oct_encode) ----------------------
float3 oct_decode(float2 e) {
    e = e * 2.0 - 1.0;                          // [0,1] -> [-1,1]
    float3 n = float3(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    float t = saturate(-n.z);
    n.x += n.x >= 0.0 ? -t : t;
    n.y += n.y >= 0.0 ? -t : t;
    return normalize(n);
}

// Multi-Paradigm Render-Path Rung-0 (Decision 3): the Cook-Torrance/GGX primitive terms
// (D_GGX / V_SmithGGXCorrelated / F_Schlick / safe_normalize) moved OUT to
// `pbr_lighting.hlsli` — a VERBATIM textual cut, character-identical to the region this
// `#include` replaces. `PI` (declared above) stays in scope for `D_GGX` (the header's
// INCLUDE CONTRACT). Permanent shared BRDF seam: later render paths `#include` the same
// header instead of duplicating these terms.
#include "pbr_lighting.hlsli"

// === Render terminator-softening — PREPROCESSOR VARIANT `TERMINATOR_WRAP` (frozen-base
// discipline, mirrors `gbuffer_mrt.fs.hlsl`'s `#ifdef`/`#else`/`#endif` convention) ===========
//
// A runtime `if (ts > 0.0) {...} else {VERBATIM original}` guard was tried first and dropped:
// even on the `ts == 0` (OFF) path, the mere PRESENCE of the extra branch + loads perturbed
// DXC's FMA fusion in the surrounding function, drifting the base resolve's pixels off the
// golden. The fix moves the choice to the PREPROCESSOR instead: with `TERMINATOR_WRAP`
// undefined (the base compile, `deferred_pbr.comp.spv`), every site below preprocesses to a
// token stream CHARACTER-IDENTICAL to the pre-feature source (the `#else` arms), so DXC's
// codegen for the base is untouched BY CONSTRUCTION — no branch, no dead code, nothing for the
// optimizer to see differently. The wrap arm carries NO `if (ts > 0.0)` runtime check (the
// variant itself IS the opt-in: the host selects `deferred_pbr_wrap.comp.spv` only when
// `LightingConfig::terminator_softening > 0`, see `deferred_pbr_wrap_spirv` in `compute.rs`).
#if TERMINATOR_WRAP
// Diffuse-terminator wrap (Valve/half-Lambert family): w=0 reduces EXACTLY to
// saturate(nol) (the byte-identity anchor); w>0 ramps the terminator over a band of
// width ~w so normal-mapped bump slopes fade into shadow instead of clipping to
// black islands. DIFFUSE ONLY — specular keeps the physical clamp.
float nol_wrapped(float nol, float w) {
    return saturate((nol + w) / (1.0 + w));
}
#endif

// Karis "mobile" analytic environment BRDF approximation (no DFG LUT). Returns the
// (scale, bias) the split-sum specular IBL needs: `spec_env = f0*scale + bias`.
float2 env_brdf_approx(float roughness, float NoV) {
    const float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
    const float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
    float4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * NoV)) * r.x + r.y;
    return float2(-1.04, 1.04) * a004 + r.zw;
}

// === PBR P1 — the HDR sun disc in the reflected environment =================================
//
// The environment the reflection vector `R` samples (the LIGHT_KIND_SKY gradient below) gets a
// bright "sun disc" baked in for every directional light — the canonical chrome cue (a sharp
// highlight sliding across a smooth metal's curvature) a flat sky-gradient alone cannot produce.
// `sun_kernel_exponent` is the cheap analytic stand-in for prefiltered-cubemap mip selection: it
// maps the GGX alpha (roughness^2) to a Blinn-Phong-equivalent cosine-power exponent
// (`n = 2/alpha^2 - 2`, the standard Phong<->GGX conversion), clamped to avoid the `pow` blowup
// as alpha -> 0 (a mirror-smooth surface) while staying a valid (if very broad) lobe as
// alpha -> 1 (fully rough). At roughness 0.13 (alpha ~= 0.0169) this clamps to the MAX (a tight
// ~1.5-degree-half-width disc); at roughness 0.42 (alpha ~= 0.1764) it lands at n ~= 62 (a broad
// ~8.5-degree glint) — the sharp-vs-soft contrast the visual gate calls for.
static const float SUN_KERNEL_EXPONENT_MIN = 1.0;
static const float SUN_KERNEL_EXPONENT_MAX = 2048.0;

float sun_kernel_exponent(float alpha) {
    float n = 2.0 / max(alpha * alpha, 1e-6) - 2.0;
    return clamp(n, SUN_KERNEL_EXPONENT_MIN, SUN_KERNEL_EXPONENT_MAX);
}

// The analytic sun-disc kernel: `pow(saturate(dot(dir, sun_dir)), k(alpha))`. `dir` is the
// REFLECTION vector `R` (not the surface normal — the disc must slide with the view, the
// mirror cue), `sun_dir` is the directional light's unit direction. Peaks at exactly 1.0 where
// `R` points at the light (a smooth metal shows a pinpoint glint there) and falls off per
// `sun_kernel_exponent` — sharp on a low-roughness surface, broad on a high-roughness one.
float sun_kernel(float3 dir, float3 sun_dir, float alpha) {
    float c = saturate(dot(dir, sun_dir));
    return pow(c, sun_kernel_exponent(alpha));
}

// The default gate on the env sun-disc contribution — owner-retunable at the visual gate.
// `1.0` keeps the disc's peak commensurate with the material's own DFG-weighted specular tint
// (the kernel itself already peaks at exactly 1.0 only where `R` points at the light and falls
// off sharply everywhere else, so a unity weight is not overbearing away from the highlight).
static const float SUN_ENV_WEIGHT = 1.0;

// === Render sky background — the visible sun disc baked into the BACKGROUND (mask == 0) =====
//
// The background branch renders the SAME analytic sky the metals reflect (`sky_kernel`
// below), so a bare pixel is coherent with the reflected environment instead of a dark void.
// `SKY_SUN_EXPONENT` is a FIXED, moderate cosine-power exponent (unlike `sun_kernel_exponent`,
// NOT roughness-driven — the background is a flat environment element, not a BRDF lobe): ~512
// reads as a tight but clearly visible sun disc against the sky gradient. Owner-retunable.
static const float SKY_SUN_EXPONENT = 512.0;

// === Render Shadow Phase 3 — SSCS screen-space march ========================================

// Interleaved-Gradient Noise (Jorge Jimenez) — a cheap per-pixel hash in [0,1), used to
// dither the SSCS start offset so the discrete step pattern reads as noise (later resolvable
// by the existing depth-aware blur) rather than banding. Deterministic in (px,py).
float ign(uint px, uint py) {
    float3 magic = float3(0.06711056, 0.00583715, 52.9829189);
    return frac(magic.z * frac(dot(float2((float)px, (float)py), magic.xy)));
}

// Projects a world point `Pm` to screen pixel coords + camera-space (view) depth. The EXACT
// INVERSE of `generate_ray` (`ray_gen.hlsli`): the perspective branch inverts the NDC→dir
// basis combine (Y-flip undone); the ortho branch inverts the linear `u/v` map. `cam_forward`
// is contractually NORMALIZED so `dot(rel, cam_forward.xyz)` is the true view depth.
//   out sx,sy  : pixel center coords (the `+0.5` of generate_ray undone via `-0.5`)
//   out view_z : camera-space depth (> 0 in front of the eye/plane)
//   return     : valid flag (view_z > 0 AND the projected pixel is in [0,w)×[0,h))
bool project_to_screen(float3 Pm, uint w, uint h, out float sx, out float sy, out float view_z) {
    if (camera_mode == RAYGEN_CAM_PERSPECTIVE) {
        float3 rel = Pm - cam_eye.xyz;
        float vz = dot(rel, cam_forward.xyz);    // camera-space depth (forward normalized)
        float vx = dot(rel, cam_right.xyz);
        float vy = dot(rel, cam_up.xyz);
        view_z = vz;
        if (vz <= 0.0) { sx = 0.0; sy = 0.0; return false; }
        float tan_half_fov = cam_forward.w;      // tan(fovY / 2)
        float aspect       = cam_right.w;        // W / H
        float ndc_x = (vx / vz) / (aspect * tan_half_fov);
        float ndc_y = (vy / vz) / tan_half_fov;
        sx = ((ndc_x * 0.5 + 0.5) * (float)w) - 0.5;
        sy = (((-ndc_y) * 0.5 + 0.5) * (float)h) - 0.5; // undo the generate_ray Y-flip
    } else {
        // ORTHO: the linear inverse of the `u/v` ray-origin map. View depth grows from the
        // RAYGEN_CAM_Z camera plane toward -Z (the ray travels (0,0,-1)).
        float u = Pm.x / RAYGEN_HALF_EXTENT;
        float v = Pm.y / RAYGEN_HALF_EXTENT;
        view_z = RAYGEN_CAM_Z - Pm.z;
        sx = ((u * 0.5 + 0.5) * (float)w) - 0.5;
        sy = (((-v) * 0.5 + 0.5) * (float)h) - 0.5; // undo the Y-flip
    }
    bool in_bounds = (sx >= 0.0) && (sy >= 0.0) && (sx <= (float)(w - 1u)) && (sy <= (float)(h - 1u));
    return (view_z > 0.0) && in_bounds;
}

// Screen-space contact-shadow march from surface point `P` (world) with normal `n` toward the
// light direction `l` (TO the light). `t_max` bounds the world march length (the to-light
// distance for a punctual caster, a big number for a directional). Returns the contact
// VISIBILITY in [0,1] (1 = fully lit, < 1 = a near occluder was found in screen space).
//
// Each `[unroll]` step advances `step = SSCS_CONTACT_LENGTH/SSCS_STEPS` world units along `l`
// (dithered by `ign`), projects the marched point to screen, reconstructs the SCENE surface
// there from the depth G-buffer via the SHARED `generate_ray` (so the depth reconstruction can
// never drift from the marcher), and compares the marched view depth to the scene view depth.
// An occluder is `0 < depth_diff < compare_tol` (a slope-scaled, thickness-floored tolerance so
// a thin sliver in front is a hit but a far background is NOT a leak). An HDRP screen-edge
// vignette + a far-distance fade taper the term to 0 where SSCS is unreliable.
float sscs_march(float3 P, float3 n, float3 l, float t_max, float NoL, uint px, uint py, uint w, uint h) {
    // Distance fade: disable SSCS on far surfaces (the screen-space depth gather is unreliable
    // and the contact gap is sub-pixel there). The center pixel's own view depth.
    float sx0, sy0, vz0;
    bool ok0 = project_to_screen(P, w, h, sx0, sy0, vz0);
    if (!ok0 || vz0 > SSCS_DISTANCE_FADE) {
        return 1.0;
    }

    // Cap the total march length to the to-light distance `t_max` (an occluder past the light
    // cannot shadow the surface), then split it into the fixed step count.
    float march_len = min(SSCS_CONTACT_LENGTH, t_max);
    float step = march_len / (float)SSCS_STEPS;

    float ign_dither = ign(px, py);
    // Normal-offset the march origin off the surface to clear self-intersection (anti-acne),
    // reusing the existing analytic-shadow bias const.
    float3 origin = P + n * SHADOW_NORMAL_BIAS;

    float occlusion = 0.0;
    [unroll]
    for (uint k = 1u; k <= SSCS_STEPS; ++k) {
        float t = ((float)k - 0.5 + ign_dither) * step;
        float3 Pm = origin + l * t;

        float sx, sy, vz_marched;
        bool valid = project_to_screen(Pm, w, h, sx, sy, vz_marched);
        if (!valid) {
            continue;                              // off-screen sample contributes 0 (edge fade)
        }

        int2 sc = int2((int)round(sx), (int)round(sy));
        float view_t_s = gViewT.Load(sc);
        if (view_t_s >= 1.0e30) {
            continue;                              // background / empty sentinel — no occluder
        }
        // Reconstruct the SCENE surface at the sampled pixel via the SHARED ray-gen (no drift),
        // then its camera-space depth (the SAME `dot(.,cam_forward)` project_to_screen uses).
        float3 ro_s, rd_s;
        generate_ray((uint)sc.x, (uint)sc.y, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro_s, rd_s);
        float3 Ps = ro_s + rd_s * view_t_s;
        float vz_scene = dot(Ps - cam_eye.xyz, cam_forward.xyz);

        float depth_diff = vz_marched - vz_scene;  // > 0 ⇒ the march is BEHIND the scene surface
        // Slope-scaled, thickness-floored tolerance: a grazing light (low NoL) needs a wider
        // window (its march steps span more depth per pixel) — without it grazing rays leak.
        float step_frac = step;
        float compare_tol = max(SSCS_THICKNESS_FLOOR, step_frac) * (1.0 + (1.0 - NoL));
        if (depth_diff > 0.0 && depth_diff < compare_tol) {
            occlusion += 1.0 / (float)SSCS_STEPS;  // a near occluder along the light ray
        }
    }

    // HDRP screen-edge vignette: fade the term out near the frame border, where a marched
    // sample leaves the screen and the gather is one-sided. `ndc` of the CENTER pixel.
    float2 ndc = float2(((float)px + 0.5) / (float)w, ((float)py + 0.5) / (float)h) * 2.0 - 1.0;
    float2 vfade = max(SSCS_EDGE_FADE_K * abs(ndc) - (SSCS_EDGE_FADE_K - 1.0), 0.0.xx);
    float edge_fade = saturate(1.0 - dot(vfade, vfade));
    occlusion *= edge_fade;

    return saturate(1.0 - occlusion);
}

// Multi-paradigm render-path plan, rung R4b (Decision 3 extraction): the output-stage tonemap
// operators (`aces_fitted`/`khronos_pbr_neutral`/`reinhard_jodie`/`tonemap_select` +
// `ACES_IN`/`ACES_OUT`/`OETF_GAMMA_EXP`) moved to `pbr_lighting.hlsli` (already `#include`d
// above) — the SAME curve every render path's final `lit` write now shares.

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    uint w = img_w();
    uint h = img_h();
    if (idx >= w * h) {
        return;
    }
    uint px = idx % w;
    uint py = idx / w;

#if SHADOW_STAGE == SHADOW_STAGE_VIS
    // Rung 3a VIS: seed the NEUTRAL visibility (full vis, validity 0) for EVERY in-bounds pixel.
    // A pixel that never reaches the mesh-shadow arm (background, `NoL <= 0`, `csm_mode == OFF`,
    // or a non-directional/`mask == 0` pixel) keeps this — validity 0 tells the à-trous filter the
    // texel carries no real sample. The mesh arm OVERWRITES this with `float2(mesh_vis, 1.0)`.
    gShadowVis[uint2(px, py)] = float2(1.0, 0.0);
#endif

    int2 coord = int2((int)px, (int)py);
    float4 albedo_texel   = gAlbedo.Load(coord);
    float4 normal_texel   = gNormal.Load(coord);
    float4 material_texel = gMaterial.Load(coord);

    float3 base = albedo_texel.rgb;             // RAW LINEAR base color
    float shadow = material_texel.r;            // A1 soft-shadow visibility [0,1]
    float ao     = material_texel.g;            // A2 ambient occlusion       [0,1]
    // `mask` is a BINARY flag (1.0 or 0.0) stored in gMaterial.b; an R8 round-trip maps
    // it to byte 255 / 0 and back to 1.0 / 0.0, so decode as `> 0.5` (robust to the LSB).
    bool is_sdf_lit = material_texel.b > 0.5;

    float3 lit;
    if (is_sdf_lit) {
        // Decode the world normal + the 16-bit material id from gNormal.
        float3 n = oct_decode(normal_texel.rg);
        uint id_lo = (uint)(normal_texel.b * 255.0 + 0.5);
        uint id_hi = (uint)(normal_texel.a * 255.0 + 0.5);
        uint mat_id = id_lo | (id_hi << 8);

        MaterialGpu m = Materials[mat_id];
        float metallic    = m.mrr.x;
        float roughness   = clamp(m.mrr.y, 0.045, 1.0); // fp32 floor (no fp16 floor needed)
        float reflectance = m.mrr.z;
#if !HWRT
        // Textured-PBR T6a (SOFTWARE-RESOLVE-ONLY): the `MATERIAL_FLAG_TEXTURED_BIT` override.
        // `reflectance` above is left UNTOUCHED (no texture channel carries it yet); `metallic`/
        // `roughness`/`ao`/`emissive` are REASSIGNED, never reordered — for a flag=0 material
        // (every current material) the branch is DEAD and every value stays a bit-for-bit copy of
        // its pre-branch assignment (the flag=0 byte-identity invariant). `ao` is the OUTER
        // `is_sdf_lit`-scope A2 SDF-march AO (declared above `is_sdf_lit`'s body start); reassigning
        // it here propagates into `ao_final` below (the SSAO combine reads THIS `ao`).
        float3 emissive = m.emissive.rgb;
        if (asuint(m.mrr.w) & MATERIAL_FLAG_TEXTURED_BIT) {
            float4 pbr = gPbr.Load(coord);
            metallic  = pbr.r;
            roughness = clamp(pbr.g, 0.045, 1.0);
            ao        = ao * pbr.b;
            emissive  = emissive * pbr.a;
        }
#endif
        float a = roughness * roughness;                 // GGX alpha = perceptual^2

        // Metallic-roughness split: dielectric f0 from reflectance (0.5 -> 4% F0); metals
        // take base as f0 and kill the diffuse lobe.
        float3 f0 = lerp(0.16 * reflectance * reflectance, base, metallic);
        float3 diffuse_color = base * (1.0 - metallic);

        // View direction: V = -ray_dir (the eye-to-surface ray reversed), from the SHARED
        // ray-gen so the resolve and marcher agree exactly. Plain IEEE ops.
        float3 ro, rd;
        generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);
        float3 v = -rd;
        float NoV = max(dot(n, v), 1e-4);

        // PBR P0-D: multi-scatter energy compensation — ONE per-pixel term (view +
        // roughness only), hoisted here (before the light loop) and REUSED at every
        // specular site below (both direct-light sites + the sky ambient specular) so
        // `env_brdf_approx` runs exactly once per pixel (Principle 1 — no duplicate ALU).
        // Single-scatter GGX loses energy at high roughness (up to ~40%), so a rough metal
        // (no diffuse fallback) came out too dark/desaturated. `Ess = dfg.x + dfg.y` is the
        // Fdez-Aguera scale+bias energy estimate (NOT `1/dfg.y`); `energy_comp` redistributes
        // the lost energy back into the metal's own f0 tint.
        float2 dfg_v = env_brdf_approx(roughness, NoV);
        float  Ess = max(dfg_v.x + dfg_v.y, 1e-4);
        float3 energy_comp = 1.0 + f0 * (1.0 / Ess - 1.0);

        // Multi-Paradigm Render-Path Rung-4a (Decision 3): the per-pixel `Surface` carrier
        // (`pbr_lighting.hlsli`) — bundles the hoisted terms every direct-light + ambient
        // evaluation below now shares via `eval_pbr_direct_bsdf`/`eval_pbr_ambient_hemi`/
        // `eval_pbr_sun_disc` instead of re-deriving the bare locals per call site.
        Surface surf;
        surf.n = n;
        surf.NoV = NoV;
        surf.a = a;
        surf.f0 = f0;
        surf.diffuse_color = diffuse_color;
        surf.energy_comp = energy_comp;

        // PBR P1: the REFLECTION vector, hoisted ONCE per pixel (view + normal only) so BOTH
        // the sky-gradient ambient specular (LIGHT_KIND_SKY) and the per-directional HDR
        // sun-disc term (LIGHT_KIND_DIRECTIONAL, below) reuse the SAME `R` — a pixel with N
        // directional lights costs one `reflect`, not N.
        float3 R = reflect(-v, n);

        // The hemisphere factor the sky lerp interpolates against (world up).
        float hemi = dot(n, LIGHT_UP) * 0.5 + 0.5;

        // L0a: loop the no-`P` front block of the table (directionals + sky). The W1
        // op-order is PINNED to the host oracle (`golden_deferred_resolve_table`):
        //   direct  += (diff + spec) * (NoL * shadow) * L.color        (accumulator from 0)
        //   ambient += diff_ambient * ao_final + spec_ambient * spec_ao (PBR metal fix:
        //              decoupled diffuse-AO/specular-AO, accumulator from 0)
        //   lit      = (direct + ambient + emissive) * exposure          (* exposure LAST)
        // No reassociation — a degenerate 1-directional + 1-sky table at exposure 1.0 is
        // bit-identical to the old LIGHT_DIR/LIGHT_COLOR/SKY_* path (pre-metal-fix).
        LightHeader H = load_light_header(LightBuf);

        // P6 R1: the resolve shadow_mode (header word 7; 0 on every pre-P6 scene → the
        // BYTE-IDENTICAL 0%-gate) + the surface world position `P` (the gViewT lane, read
        // here under `mask == 1` — the SAME reconstruction the L0b block uses below, hoisted
        // up so the directional march can use it too). The `marched` counter bounds the
        // per-pixel march to `MAX_SDF_SHADOW_CASTERS_PER_PIXEL` dominant casters (Decision 2).
        uint shadow_mode = load_shadow_mode(LightBuf);
        bool multi_light = shadow_mode != SHADOW_MODE_LEGACY;
        // Render Shadow Phase 3: the SSCS gate (header word 7 bit 1; OFF on every pre-Phase-3
        // scene → the per-light `sscs_march` block never runs → byte-identical to today). Read
        // ONCE here, alongside `ssao_mode`, then consumed at both `vis` sites below.
        uint contact_mode = load_contact_shadow_mode(LightBuf);
        // CSM Increment 1b (Rung A): the cascade-shadow gate (header word 7 bit 2; OFF on every
        // pre-CSM scene → the `csm_visibility` sample never runs → the bound-but-unread cascade
        // map/sampler/UBO are never sampled → byte-identical to today, the 0%-gate). Read ONCE
        // here, consumed at the primary-directional `vis` site below.
        uint csm_mode = load_csm_mode(LightBuf);
        // Shadow Phase 5 Inc-1-GPU: the sparse SPOT/POINT atlas gate (header word 7 bit 3; OFF on
        // every pre-Inc-1 scene → the per-spot `spot_atlas_visibility` sample never runs → the
        // bound-but-unread atlas map/sampler/UBO are never sampled → byte-identical to today, the
        // 0%-gate). Read ONCE here, consumed at the per-spot `vis` site in the point/spot loop below.
        uint punctual_shadow_mode = load_punctual_shadow_mode(LightBuf);
        // SDFDDGI I0: the DDGI (SDF diffuse GI) gate (header word 7 bit 4; OFF on every pre-SDFDDGI
        // scene → the probe-irradiance injection block never runs → the bound-but-unread DDGI
        // irradiance/depth/UBO bindings (16/17/18) are never sampled → byte-identical to today, the
        // 0%-gate). Read ONCE here, consumed at the GATED (empty at I0) injection site after the L0a
        // ambient accumulation below.
        uint ddgi_mode = load_ddgi_mode(LightBuf);
        // Render terminator-softening (`#if TERMINATOR_WRAP` variant — the frozen-base
        // discipline note above `nol_wrapped`): the diffuse light-wrap amount, header word 7
        // bits 12..19. Read ONCE here (a wave-uniform header broadcast), consumed at each
        // direct-light diffuse accumulation site below via `nol_wrapped` (DIFFUSE ONLY —
        // specular keeps the physical NoL clamp). Declared ENTIRELY under `TERMINATOR_WRAP` so
        // the base compile references neither `ts` nor `load_terminator_softening` (the latter
        // stays an unreferenced, harmless function in the shared `light_table.hlsli`).
#if TERMINATOR_WRAP
        float ts = load_terminator_softening(LightBuf);
#endif
        float view_t = gViewT.Load(coord);
        float3 P = ro + rd * view_t;
#ifdef MOTION_VECTORS
        // Rung 3b step 5b: the SDF pixel's CAMERA-ONLY motion vector. Reproject the reconstructed
        // world surface `P` through the previous + current view-proj (SDF-edit motion deferred).
        // Written here — inside `is_sdf_lit`, right after `P` and BEFORE the light loop — so EVERY
        // SDF pixel gets exactly one write. Mesh pixels are raster-owned (the gbuffer MV variant
        // wrote them); the two producers cover DISJOINT pixels of one `motion_vec`. Static camera ⇒
        // `mv_prev == mv_cur` ⇒ Δuv = 0.
        gMotionVec[uint2(px, py)] =
              mv_clip_to_uv(mul(mv_prev_view_proj, float4(P, 1.0)))
            - mv_clip_to_uv(mul(mv_cur_view_proj, float4(P, 1.0)));
#endif
        uint marched = 0u;

        // Render P7: the SSAO combine (a structural `if`, the 0%-gate). `ao` is the A2 SDF
        // march (`gMaterial.g`). When `ssao_mode == 0` (every pre-P7 scene) `ao_final == ao`
        // and `gSsao` is never read → arithmetically byte-identical to today. When armed, the
        // per-pixel class uses the ALREADY-loaded `view_t` (no extra fetch): a mesh pixel
        // (`view_t >= 1e30` sentinel) has NO field AO so it takes pure SSAO; an SDF pixel keeps
        // the exact march unless SSAO sees a cross-representation occluder (`min` — most-occluded
        // wins). (The SSAO PASS that writes `gSsao` is Render P7 GROUP C2; on a `ssao_mode == 0`
        // scene the image is never read, so its undefined contents are irrelevant.)
        float ao_final = ao;
        uint ssao_mode = load_ssao_mode(LightBuf);
        if (ssao_mode != SSAO_MODE_OFF) {
            // Render P7 POLISH follow-up: the SSAO denoise now lives in a DEDICATED multi-pass
            // à-trous compute chain (`ssao_atrous.comp.hlsl`), run BEFORE this resolve dispatch.
            // `gSsao` here is the chain's FINAL filtered lane (or the raw `sdf_ssao` gather when
            // the denoise is off, `atrous_levels == 0`) — a single `.Load`, no per-pixel
            // neighbourhood walk in the resolve anymore.
            //
            // Render P7 POLISH Track 2: the tail combine below is MACHINE-GENERATED by
            // `boyko_shaderdsl::emit::emit_hlsl_ssao_blur_combine()` (tracing
            // `boyko_shaderdsl::ssao::ssao_blur_combine_body` over the Emit backend) — the SAME
            // single-sourced body the host oracle `golden_ssao_atrous` / `ssao_blur_combine_body`
            // locks against. The `ssao_atrous_edsl_sync` test pins the committed span to the
            // generator; a hand-edit fails CI.
            float ssao_blurred = gSsao.Load(coord).r;
            // === GENERATED ssao_blur_combine BEGIN ===
            float ao_class = (view_t >= 1.0e30) ? 1.0 : ao;
            ao_final = min(ao_class, ssao_blurred);
            // === GENERATED ssao_blur_combine END ===
        }

        // PBR metal fix: decoupled specular occlusion (Filament SpecularAO_Lagarde ==
        // Bevy deferred `specular_occlusion`), hoisted ONCE per pixel (NoV + roughness +
        // ao_final only) and reused at every ambient-specular site below. Diffuse AO
        // (ao_final) darkens Lambert ambient correctly, but a metal has diffuse == 0 — its
        // ambient SPECULAR is its ENTIRE appearance, so multiplying that by ao_final reads
        // as "AO-darkened matte paint", not metal. `spec_ao` stays ~1 for smooth/metal
        // surfaces and only gently occludes rough+cavity, matching every competitor's split.
        float spec_ao = saturate(pow(NoV + ao_final, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao_final);

        float3 lit_direct = float3(0.0, 0.0, 0.0);
        float3 ambient = float3(0.0, 0.0, 0.0);
        bool primary_dir_seen = false;
        for (uint i = 0u; i < H.l0a_count; ++i) {
            LightElem L = load_light(LightBuf, i);
            if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
                float3 l = normalize(L.dir);
                float NoL = max(dot(n, l), 0.0);
                // P6 R1: the primary directional (the FIRST directional — the one the marcher
                // marched into `gMaterial.r`, Decision 6) KEEPS `gMaterial.r` in all modes
                // (never re-marched). EXTRA directionals DEFAULT to `shadow` (the legacy L0a
                // modulation — every directional multiplied by gMaterial.r today), so a
                // `shadow_mode==0` scene is BYTE-IDENTICAL to today (0%-gate). In multi-light
                // mode an extra FLAGGED directional instead gets a `t_max=T_MAX` analytic
                // march (it reaches everywhere — unbounded, capped by dominant-N + NoL skip).
                float vis = shadow;
                if (!primary_dir_seen) {
                    primary_dir_seen = true;      // the primary KEEPS gMaterial.r (vis=shadow)
                    // CSM Increment 1b (Rung A): MIN-COMBINE the cascade hard-shadow into the
                    // primary directional's analytic visibility. The exact raster-mesh shadow
                    // (a hardware depth-map PCF) and the analytic SDF term are independent
                    // occluders — `min` keeps the MOST-occluded (a pixel shadowed by EITHER is
                    // shadowed). Gated by the header bit + a front-facing receiver (a
                    // back-faced surface is already `NoL == 0`, so the cascade lookup would be
                    // wasted). OFF on every pre-CSM scene → byte-identical (the 0%-gate).
                    if (csm_mode != CSM_MODE_OFF && NoL > 0.0) {
                        // CSM Increment 3 (Rung B): the cascade SELECT needs the receiver's
                        // VIEW-SPACE depth — the SAME quantity the L1 froxel slice uses (`dot(rd,
                        // cam_forward) * view_t` for PERSP, `view_t` for ORTHO; `cam_forward.xyz` is
                        // contractually NORMALIZED, O1). The PSSM `split_far` boundaries are
                        // view-space too, so the SELECT runs in VIEW-Z LINEAR space.
                        float csm_view_z = (camera_mode == RAYGEN_CAM_PERSPECTIVE)
                                         ? (dot(rd, cam_forward.xyz) * view_t)
                                         : view_t;
#if HWRT
    #if SHADOW_STAGE == SHADOW_STAGE_RESOLVE_INLINE
                        // R2a-4b (owner-eval, soft): the mesh-shadow term routes to a SOFT `rayQuery`
                        // TLAS trace (replacing the CSM shadow-map sample for mesh geometry). The
                        // directional light is at infinity, so the shadow rays cast toward `l` (the
                        // world dir TO the light) from `P + n * bias`. Instead of ONE ray (too sharp),
                        // `SHADOW_RAY_COUNT` rays are jittered on a Vogel disk within the sun's angular
                        // cone (`SHADOW_CONE_RADIUS` = tan(half-angle)) around `l`; the AVERAGE miss
                        // fraction is the soft penumbra. The SDF analytic term (already in `vis`) stays
                        // min-combined. Flags per ray: ACCEPT_FIRST_HIT_AND_END_SEARCH (occlusion query
                        // — first hit suffices), FORCE_OPAQUE (no any-hit) + SKIP_PROCEDURAL_PRIMITIVES
                        // (the BLAS is triangles-only; skip any AABB geometry).
                        //
                        // Orthonormal basis around `l` (guard the near-parallel `up` case); the
                        // per-pixel golden-angle spiral is rotated by the shader's own IGN hash (the
                        // SAME `ign(px, py)` the SSCS dither uses) so neighbouring pixels sample
                        // decorrelated cone directions — the penumbra reads as noise, not banding (no
                        // spatial TAA on this engine, so `SHADOW_RAY_COUNT` carries the single-frame
                        // smoothness). Rung 3b adds a SECOND, per-frame rotation term keyed off
                        // `SHADOW_FRAME_SEED`: the spiral also advances by the golden angle every
                        // frame, so the same pixel samples a DIFFERENT cone subset frame-to-frame —
                        // the temporal decorrelation the shadow temporal-reproject pass averages out.
                        float3 sh_up = abs(l.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0);
                        float3 sh_tx = normalize(cross(sh_up, l));
                        float3 sh_ty = cross(l, sh_tx);
                        float  sh_rot = ign(px, py) * 6.2831853 + float(SHADOW_FRAME_SEED & 0xFFu) * 2.399963229728653; // IGN spiral + rung-3b per-frame golden-angle step
                        float  occ = 0.0;
                        [loop] for (uint si = 0u; si < SHADOW_RAY_COUNT; ++si) {
                            float sh_r = sqrt((si + 0.5) / SHADOW_RAY_COUNT);        // Vogel disk radius
                            float sh_t = si * 2.399963229728653 + sh_rot;            // golden angle + IGN
                            float2 sh_d = float2(cos(sh_t), sin(sh_t)) * (sh_r * SHADOW_CONE_RADIUS);
                            float3 sh_dir = normalize(l + sh_tx * sh_d.x + sh_ty * sh_d.y);
                            RayDesc shadow_ray;
                            shadow_ray.Origin = P + n * SHADOW_RAY_BIAS;
                            shadow_ray.Direction = sh_dir;
                            shadow_ray.TMin = SHADOW_RAY_TMIN;
                            shadow_ray.TMax = SHADOW_RAY_TMAX;
                            RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH
                                   | RAY_FLAG_FORCE_OPAQUE
                                   | RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES> q;
                            q.TraceRayInline(tlas, 0, 0xFF, shadow_ray);
                            q.Proceed();
                            occ += (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT) ? 1.0 : 0.0;
                        }
                        float mesh_vis = 1.0 - occ / SHADOW_RAY_COUNT;
                        vis = min(vis, mesh_vis);
    #elif SHADOW_STAGE == SHADOW_STAGE_VIS
                        // Rung 3a VIS: the IDENTICAL Vogel-disk trace as RESOLVE_INLINE (same
                        // SHADOW_RAY_COUNT spec-const, cone/tmax/tmin/bias UBO, IGN rotation,
                        // golden angle, rung-3b SHADOW_FRAME_SEED per-frame rotation term, ray
                        // flags) — copied VERBATIM so `mesh_vis` is bit-identical to the inline
                        // path, INCLUDING across frames now that the rotation is frame-varying (the
                        // C3 algebraic anchor). The ONLY divergence vs
                        // RESOLVE_INLINE is the SINK: instead of `vis = min(vis, mesh_vis)`, write
                        // the raw visibility (+ validity 1) to `gShadowVis` and RETURN before any
                        // lighting — the VIS stage produces NO lit output (the à-trous filter + the
                        // RESOLVE_DENOISED stage consume `gShadowVis`).
                        float3 sh_up = abs(l.y) < 0.99 ? float3(0.0, 1.0, 0.0) : float3(1.0, 0.0, 0.0);
                        float3 sh_tx = normalize(cross(sh_up, l));
                        float3 sh_ty = cross(l, sh_tx);
                        float  sh_rot = ign(px, py) * 6.2831853 + float(SHADOW_FRAME_SEED & 0xFFu) * 2.399963229728653; // IGN spiral + rung-3b per-frame golden-angle step
                        float  occ = 0.0;
                        [loop] for (uint si = 0u; si < SHADOW_RAY_COUNT; ++si) {
                            float sh_r = sqrt((si + 0.5) / SHADOW_RAY_COUNT);        // Vogel disk radius
                            float sh_t = si * 2.399963229728653 + sh_rot;            // golden angle + IGN
                            float2 sh_d = float2(cos(sh_t), sin(sh_t)) * (sh_r * SHADOW_CONE_RADIUS);
                            float3 sh_dir = normalize(l + sh_tx * sh_d.x + sh_ty * sh_d.y);
                            RayDesc shadow_ray;
                            shadow_ray.Origin = P + n * SHADOW_RAY_BIAS;
                            shadow_ray.Direction = sh_dir;
                            shadow_ray.TMin = SHADOW_RAY_TMIN;
                            shadow_ray.TMax = SHADOW_RAY_TMAX;
                            RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH
                                   | RAY_FLAG_FORCE_OPAQUE
                                   | RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES> q;
                            q.TraceRayInline(tlas, 0, 0xFF, shadow_ray);
                            q.Proceed();
                            occ += (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT) ? 1.0 : 0.0;
                        }
                        float mesh_vis = 1.0 - occ / SHADOW_RAY_COUNT;
                        // Overwrite the neutral seed: R = the raw mesh visibility, G = 1 (a real
                        // mesh-shadow sample). Exactly ONE gShadowVis texel per pixel is written.
                        gShadowVis[uint2(px, py)] = float2(mesh_vis, 1.0);
    #else // SHADOW_STAGE_RESOLVE_DENOISED
                        // Rung 3a DENOISED: replace the inline trace with a single point-read of the
                        // à-trous-FILTERED visibility (the host binds gShadowVis to the final à-trous
                        // level here), then the IDENTICAL min-combine at the IDENTICAL predicate as
                        // RESOLVE_INLINE. With a pass-through filter (levels == 0) `mesh_vis` equals
                        // the VIS write, which equals the inline `mesh_vis`, so the DENOISED render is
                        // bit-identical to RESOLVE_INLINE (the C3 algebraic anchor: `af934c50`).
                        float mesh_vis = gShadowVis.Load(int2(px, py)).r;
                        vis = min(vis, mesh_vis);
    #endif
#else
                        vis = min(vis, csm_visibility(P, n, csm_view_z, NoL));
#endif
                    }
                } else if (multi_light && light_casts_sdf_shadow(L)
                           && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
                           && NoL > SHADOW_NDOTL_EPS) {
                    // Normal-offset start bias: lift the march origin off the surface so
                    // grazing rays clear it (anti terminator-acne). Mirrors the host `pb`.
                    vis = sdf_soft_shadow_ranged(P + n * SHADOW_NORMAL_BIAS, n, l, T_MAX);
                    marched += 1u;
                }
                // Render Shadow Phase 3: multiply the screen-space CONTACT factor into `vis`
                // (the directional caster reaches everywhere — a big `t_max`). Gated by the
                // header bit + a front-facing surface; OFF on every pre-Phase-3 scene.
                if (contact_mode == CONTACT_SHADOW_MODE_ON && NoL > 0.0) {
                    vis *= sscs_march(P, n, l, T_MAX, NoL, px, py, w, h);
                }
                // PBR P0-A.1: safe_normalize parity with the punctual site below — the
                // intrinsic `normalize(0)` is NaN when `v` is near-opposite `l` (a
                // near-grazing/back-facing directional receiver); NoL then zeroes the NaN
                // spec's contribution on the host but not on the GPU (NaN * 0 == NaN).
                // Multi-Paradigm Render-Path Rung-4a (Decision 3): the D/V/F/spec/diff
                // Cook-Torrance combination is the SHARED `eval_pbr_direct_bsdf`
                // (`pbr_lighting.hlsli`) — identical body to the point/spot site below, now
                // single-sourced.
                PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
                // Render terminator-softening (`#if TERMINATOR_WRAP` variant — the frozen-base
                // discipline note above `nol_wrapped`): DIFFUSE ONLY, `spec` keeps the physical
                // NoL clamp. No runtime check (the variant itself is the opt-in); the `#else`
                // arm is CHARACTER-FOR-CHARACTER the pre-feature statement, so the base
                // compile's token stream — and DXC's codegen for this whole function — is
                // untouched by construction.
#if TERMINATOR_WRAP
                lit_direct += (bsdf.diffuse * nol_wrapped(NoL, ts) + bsdf.specular * NoL) * vis * L.color;
#else
                lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * vis) * L.color;
#endif

                // PBR P1 — the HDR sun disc: a SECOND, roughness-widened specular response
                // from this SAME directional light, sampled along the REFLECTION vector `R`
                // (not `l`) instead of the direct Cook-Torrance half-vector lobe above. Uses
                // the SAME DFG + energy_comp weighting the sky ambient specular uses (P0-B/
                // P0-D), so the glint shades consistently with the flat-gradient reflection.
                // INTENTIONAL double-count with the direct lobe (both widen with roughness
                // together — the industry-standard sky-cubemap + directional-light overlap;
                // real-time engines carry the sun both ways). NOT shadow/vis-modulated (only
                // AO-gated, mirroring the sky ambient's own AO gate) — this is an environment
                // term, not a direct-light term. `SUN_ENV_WEIGHT` is the single tuning knob.
                // Rung-4a: the DFG/energy_comp weighting is the SHARED `eval_pbr_sun_disc`
                // (`pbr_lighting.hlsli`); `SUN_ENV_WEIGHT` stays here (declared AFTER this
                // file's `#include "pbr_lighting.hlsli"`, so the header cannot reference it).
                float sun_k = sun_kernel(R, l, a);
                float3 sun_spec_ambient = eval_pbr_sun_disc(surf, dfg_v, sun_k, L.color) * SUN_ENV_WEIGHT;
                // PBR metal fix: sun_spec_ambient is a SPECULAR term (the environment glint) —
                // decoupled from diffuse ao_final onto spec_ao (see the `spec_ao` doc above).
                ambient += sun_spec_ambient * spec_ao;
            } else if (light_kind(L) == LIGHT_KIND_SKY) {
                // Hemisphere ambient: diffuse integrates the FULL hemisphere around N
                // (Lambert), so `hemi_color = lerp(ground, sky, hemi)` keeps sampling along
                // N. PBR P0-B: the specular lobe is narrow and view-dependent — a metal must
                // MIRROR its surroundings, not show the identical flat sky tint from every
                // angle — so the specular term samples the SAME sky/ground gradient along
                // the REFLECTION vector `R` (PBR P1: hoisted above the light loop) instead of N.
                // Rung-4a: the full combination (hemi lerp + steepened reflected hemisphere +
                // decoupled-AO combine, the PBR metal fix) is the SHARED `eval_pbr_ambient_hemi`
                // (`pbr_lighting.hlsli`).
                float3 sky_color = L.color;       // upper hemisphere
                float3 ground_color = L.pos;      // lower hemisphere (packed in pos lane)
                ambient += eval_pbr_ambient_hemi(surf, R, dfg_v, sky_color, ground_color, hemi, ao_final, spec_ao);
            }
            // Point/spot (kinds 1/2) are the L0b block — not in the L0a front block.
        }

        // SDFDDGI I3 — the GATED probe-irradiance injection (GI first becomes VISIBLE here). The
        // real trilinear + wrap + Chebyshev probe sample (`ddgi_resolve.hlsli::ddgi_probe_sample`,
        // the op-for-op mirror of `goldens::probe_sample`), ADDED to the `ambient` accumulator so
        // the indirect diffuse rides on top of the L0a hemisphere/sky term.
        //
        // 0%-GATE (the CSM/punctual/SSAO precedent): the 3 DDGI bindings (16/17/18) are read ONLY
        // inside this `if (ddgi_mode != 0u)` structural gate. On the OFF path (`ddgi_mode == 0`, the
        // DEFAULT — every pre-SDFDDGI scene) the block NEVER runs, so the bindings stay bound-but-
        // UNREAD and the lit pixels are byte-identical to today. The block is REAL code reachable at
        // runtime, so DXC keeps the descriptor references (the "the .spv statically references the
        // binding" layout contract).
        //
        // Grid params from the b18 UBO: origin.xyz, inv_spacing (gDdgiInvSpacDims.x), and the three
        // u32 dims bit-cast into gDdgiInvSpacDims.yzw. The sky-ambient FALLBACK (receiver outside the
        // grid AABB / all corners unconverged) is ZERO: GI here is an ADDITIVE indirect term, so no
        // coverage adds no extra indirect (the L0a hemisphere/sky ambient already supplies the base).
        // The indirect diffuse is `diffuse_color * gi * ao_final` (the same diffuse-color × AO shaping
        // the L0a diffuse ambient uses), so metals (diffuse_color == 0) receive no GI diffuse.
        if (ddgi_mode != 0u) {
            uint3 ddgi_dims = uint3(asuint(gDdgiInvSpacDims.y),
                                    asuint(gDdgiInvSpacDims.z),
                                    asuint(gDdgiInvSpacDims.w));
            float3 gi = ddgi_probe_sample(
                P, n,
                gDdgiOrigin.xyz, gDdgiInvSpacDims.x, ddgi_dims,
                float3(0.0, 0.0, 0.0),               // additive fallback: no coverage -> no extra indirect
                gDdgiIrr, gDdgiIrrSamp,
                gDdgiDepth, gDdgiDepthSamp);
            ambient += diffuse_color * gi * ao_final;
        }

        // L0b: loop the point/spot block `[l0a_count .. light_count)`. The surface world
        // position `P` (the gViewT lane reconstruction) was hoisted to the top of this
        // `is_sdf_lit` branch (P6 R1) — the gViewT.Load still executes STRICTLY inside the
        // `mask == 1` branch (C2 read-under-mask gate; a non-lit pixel's `1.0e30` sentinel is
        // never consumed). `rd` is unit (the shared ray-gen), so `view_t` is the true world
        // distance and `P = ro + rd * view_t` is the exact marched surface point.

        // L1 cluster lookup (Decision 6): when `clusters_enabled`, map this pixel to its
        // froxel and loop ONLY the cluster's point/spot indices; else loop the flat
        // `[l0a_count .. light_count)` block (the L0b path — the L1 0%-gate). The froxel z
        // slice uses the SAME view-z the cull used: `view_z = dot(rd, cam_forward.xyz) *
        // view_t` (PERSP; cam_forward.xyz is contractually NORMALIZED, O1) or `view_t`
        // (ORTHO). The linearization (`cluster_linear_index`) + the slice/tile maps are the
        // shared `light_table.hlsli` helpers — byte-identical to the cull WRITE (a mismatch
        // would silently map to the wrong cluster).
        //
        // Two DEFENCE terms beyond the enabled bit, bringing this site to the same three-term
        // form `vb_resolve.comp.hlsl`/`vb_shade.comp.hlsl` carry (it had NEITHER of them until
        // now, and it is reachable on EVERY Deferred boot, unlike the `#ifdef FROXEL` VB sites):
        //
        //   * non-zero dims (VB-P1b-0 C1). `cluster_z_slice` clamps to `(int)dim_z - 1`, which
        //     for `dim_z == 0` is `-1` and returns `0xFFFFFFFF`; `cluster_linear_index` then
        //     yields `0xFFFFFFFF` and `ClusterGrid[cluster]` reads gigabytes past the end. A
        //     zero-dims header is exactly what `sync_cluster_light_gate` publishes on every
        //     non-VB-froxel boot -- including this shader's own Deferred path -- so the enabled
        //     bit alone was never a sufficient guard here.
        //   * CAPACITY (VB-P1k). `cluster_linear_index` is < dim_x*dim_y*dim_z by construction,
        //     so the LIVE header's dims are the only bound this read would otherwise have --
        //     while `ClusterGrid` was SIZED at boot from `ClusterConfig::cluster_count()` and is
        //     never re-allocated, and `sync_cluster_light_gate` republishes the LIVE dims every
        //     frame. A post-boot `ClusterConfig` edit that GROWS the grid therefore makes this
        //     read leave the allocation, silently (`robustBufferAccess` is OFF, no GPU-assisted
        //     validation). `GetDimensions` reports the BOUND DESCRIPTOR's own element count
        //     (SPIR-V `OpArrayLength`) -- the allocation itself, not a host-side mirror of it --
        //     so this term disarms the cluster walk for exactly the frames whose live grid does
        //     not fit the buffer, falling back to the in-bounds flat scan (which is also the
        //     CORRECT lighting for such a frame; clamping the index would silently shade against
        //     the wrong froxel).
        //
        // Both terms are inert on every armed, correctly-packed frame (dims nonzero together
        // with the enabled bit, and `cluster_count == grid_capacity` when boot dims == live
        // dims -- every shipping configuration), so ON==OFF equality is unaffected. On a
        // Deferred/ForwardPlus boot with no L1 cull built, `ClusterGrid` is bound to the light
        // table as a placeholder (`targets.rs`); `GetDimensions` then reports THAT buffer's
        // element count, which is still a true bound on the bound descriptor -- and the dims
        // term has already disarmed the walk there anyway.
        ClusterParams cp = load_cluster_params(LightBuf);
        uint grid_capacity, grid_stride;
        ClusterGrid.GetDimensions(grid_capacity, grid_stride);
        uint cluster_count = cp.dim_x * cp.dim_y * cp.dim_z;
        bool use_clusters = (cp.clusters_enabled != 0u) && (cluster_count != 0u)
                         && (cluster_count <= grid_capacity);
        uint ps_count;       // number of point/spot lights to walk
        uint ps_offset;      // base into LightIndexList (clusters) or the flat block
        if (use_clusters) {
            float view_z = (camera_mode == RAYGEN_CAM_PERSPECTIVE)
                         ? (dot(rd, cam_forward.xyz) * view_t)
                         : view_t;
            uint2 tile = cluster_xy_tile(px, py, w, h, cp);
            uint zsl = cluster_z_slice(view_z, cp);
            uint cluster = cluster_linear_index(tile.x, tile.y, zsl, cp.dim_x, cp.dim_z);
            uint2 cell = ClusterGrid[cluster];
            ps_offset = cell.x;  // offset into LightIndexList
            ps_count = cell.y;   // count of indices in this froxel's slice
        } else {
            ps_offset = H.l0a_count;                  // flat block base
            ps_count = H.light_count - H.l0a_count;   // flat block length
        }
        for (uint jj = 0u; jj < ps_count; ++jj) {
            // The light table index: the cluster's index-list entry (L1) or the flat block
            // index (L0b). The BRDF body below is UNCHANGED from L0b.
            uint j = use_clusters ? LightIndexList[ps_offset + jj] : (ps_offset + jj);
            // No index-range guard: the cull pass (`golden_cluster_cull`, untouched) only
            // ever pushes POINT/SPOT indices in `[l0a_count, light_count)` into
            // LightIndexList, exactly like the host's `grid[cluster]` Vec, so `j` is always a
            // valid point/spot slot. The prior `if (j < l0a_count || j >= light_count)`
            // guard was WRONG (it never fired for the offending light — the residual NaN
            // comes from an IN-RANGE valid light's `normalize(v+l)` at a back-facing surface,
            // fixed by `safe_normalize`) and it perturbed DXC's dead-code elimination of
            // bindings 8/9, breaking the non-clustered resolve's binding interface.
            LightElem L = load_light(LightBuf, j);
            // toL = light position - surface; d2 = squared distance for the range cull +
            // the smooth windowed inverse-square attenuation (Decision 2 / Algorithm C).
            float3 toL = L.pos - P;
            float d2 = dot(toL, toL);
            float range2 = L.range * L.range;
            if (d2 > range2) {
                continue;                            // outside the cull sphere (range)
            }
            float inv_d = rsqrt(max(d2, 1e-8));
            float3 l = toL * inv_d;
            // Smooth windowed inverse-square: 1/max(d2,eps) * window(d2/range2), where the
            // window `(1 - (d2/range2)^2)^2` (clamped) drives the contribution smoothly to
            // 0 at the cull radius so the range cutoff is bandless (the canonical UE4/
            // Frostbite falloff). eps avoids the singularity at the light.
            float win = saturate(1.0 - (d2 * d2) / (range2 * range2));
            float atten = (1.0 / max(d2, 1e-4)) * win * win;
            if (light_kind(L) == LIGHT_KIND_SPOT) {
                // O2 cone falloff: cos of the angle between the surface->light dir reversed
                // (i.e. light->surface = -l) and the spot axis, smoothstepped between the
                // outer and inner cone cosines, squared for a soft edge.
                float2 cones = unpack_cones(L.cone_pack); // (cos_inner, cos_outer)
                float3 spot_dir = safe_normalize(L.dir);  // world spot SHINE axis (cosA = dot(-l, axis))
                float cosA = dot(-l, spot_dir);
                float denom = max(cones.x - cones.y, 1e-4);
                float tt = saturate((cosA - cones.y) / denom);
                atten *= tt * tt;
            }
            // Shadow Phase 5 Inc-1/2-GPU: the punctual atlas hard-shadow term. Gated by the header
            // bit (`punctual_shadow_mode`) AND a real assigned slot (`light_atlas_slot(L.kind) !=
            // SLOT_NONE`). A SPOT (Inc 1) samples its single perspective layer via NDC-z; a POINT
            // (Inc 2) reads its slot BASE `b` then major-axis-selects one of the six cube faces
            // `b..b+6` and does a LINEAR-DISTANCE compare. The exact raster-mesh shadow (a hardware
            // depth-map PCF) modulates THIS light's contribution multiplicatively (the visibility of
            // the punctual's direct light at `P`). OFF on every pre-Inc-1 scene → the bound-but-unread
            // atlas is never sampled → byte-identical (the 0%-gate). Pass the RAW kind word `L.kind`
            // (NOT `light_kind(L)`) so the slot field survives the unpack.
            float punctual_shadow = 1.0;
            if (punctual_shadow_mode != PUNCTUAL_SHADOW_MODE_OFF) {
                uint slot = light_atlas_slot(L.kind);
                if (slot != SLOT_NONE) {
                    // The receiver's NoL for THIS punctual light (`l` = surface->light); drives the
                    // slope-scaled normal offset that suppresses grazing self-shadow acne.
                    float pnol = max(dot(n, l), 0.0);
                    if (light_kind(L) == LIGHT_KIND_SPOT) {
                        punctual_shadow = spot_atlas_visibility(slot, P, n, pnol);
                    } else if (light_kind(L) == LIGHT_KIND_POINT) {
                        // `slot` is the cube's slot BASE `b`; the six faces are `b..b+6`.
                        punctual_shadow = punctual_atlas_visibility(slot, P, n, pnol);
                    }
                }
            }
            // The SAME Cook-Torrance direct term as the directional path, scaled by the
            // distance/cone attenuation and the light's canonical (baked-I) color.
            // Multi-Paradigm Render-Path Rung-4a (Decision 3): the SHARED
            // `eval_pbr_direct_bsdf` (`pbr_lighting.hlsli`) — identical body to the
            // directional site above, including the `safe_normalize` half-vector parity
            // (host `v_normalize`; at a back-facing surface `v + l` can be ~zero, and the
            // intrinsic `normalize(0) == NaN` would poison `spec` and, since
            // `NaN * (NoL == 0) == NaN`, blacken the pixel).
            float NoL = max(dot(n, l), 0.0);
            PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
            // P6 R1: `vis` DEFAULTS to `shadow` (the marcher's gMaterial.r channel) — the
            // EXACT legacy L0b/L1 point/spot modulation (`(NoL * shadow) * atten`), so a
            // `shadow_mode==0` scene is BYTE-IDENTICAL to today (the 0%-gate; the L0b/L1
            // goldens are preserved). In multi-light mode a FLAGGED caster instead gets a
            // RANGE-BOUNDED analytic march — `t_max` is the light DISTANCE (`sqrt(d2)`, the
            // common short/cheap nearby case) so the shadow ray stops AT the light (an
            // occluder past the light cannot shadow). Gated by the per-light flag + the
            // dominant-N cap + the NoL > 0 skip (a back-faced light marches nothing).
            // Render Shadow fix: a punctual light's visibility must be its OWN shadow, never the
            // directional's `shadow` (gMaterial.r = the marcher's analytic SUN shadow). When the
            // punctual atlas is armed (`punctual_shadow_mode != OFF`) start FULLY LIT and let
            // `punctual_shadow` (the cube/spot map, multiplied in at the accumulate below) be the
            // only occlusion — otherwise the sun's analytic shadow zeroes the point on exactly the
            // hemisphere it should light (the SDF spheres "behind the light" went dark). OFF scenes
            // keep `vis = shadow` byte-for-byte (the 0%-gate; the host oracle uses `shadow` too).
            float vis = (punctual_shadow_mode != PUNCTUAL_SHADOW_MODE_OFF) ? 1.0 : shadow;
            if (multi_light && light_casts_sdf_shadow(L)
                && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
                && NoL > SHADOW_NDOTL_EPS) {
                float t_max = sqrt(d2);
                // Normal-offset start bias (anti grazing-acne). Mirrors the host `pb`.
                vis = sdf_soft_shadow_ranged(P + n * SHADOW_NORMAL_BIAS, n, l, t_max);
                marched += 1u;
            }
            // Render Shadow Phase 3: the screen-space CONTACT factor — `t_max` is the to-light
            // DISTANCE (`sqrt(d2)`) so the contact ray stops AT the light (an occluder past the
            // light cannot shadow). Gated by the header bit + a front-facing surface.
            if (contact_mode == CONTACT_SHADOW_MODE_ON && NoL > 0.0) {
                vis *= sscs_march(P, n, l, sqrt(d2), NoL, px, py, w, h);
            }
            // Render terminator-softening (`#if TERMINATOR_WRAP` variant — the frozen-base
            // discipline note above `nol_wrapped`): DIFFUSE ONLY, `spec` keeps the physical NoL
            // clamp. The `#else` arm is CHARACTER-FOR-CHARACTER the pre-feature statement (the
            // base compile's token stream is untouched by construction).
#if TERMINATOR_WRAP
            lit_direct += (bsdf.diffuse * nol_wrapped(NoL, ts) + bsdf.specular * NoL) * (vis * punctual_shadow) * atten * L.color;
#else
            lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * vis * punctual_shadow) * atten * L.color;
#endif
        }

        // O3: exposure is the FINAL multiply on the accumulated LINEAR radiance, THEN the
        // PBR P0-C filmic tonemap (hue-preserving highlight rolloff), THEN the manual gamma
        // OETF (see `aces_fitted`'s doc comment for why the OETF is manual, not hardware).
        // Textured-PBR T6a: the SOFTWARE arm sums the (possibly texture-modulated) local
        // `emissive`; the HWRT arm is CHARACTER-FOR-CHARACTER the pre-T6a line (no `emissive`
        // local exists under `HWRT` — the HWRT `.spv` is untouched by this rung).
#if !HWRT
        lit = (lit_direct + ambient + emissive) * H.exposure;
#else
        lit = (lit_direct + ambient + m.emissive.rgb) * H.exposure;
#endif
        lit = tonemap_select(lit, load_tonemap_mode(LightBuf));
        lit = pow(lit, OETF_GAMMA_EXP);
    } else {
        // mesh / background / empty (mask == 0): render the PROCEDURAL SKY along the view
        // ray instead of the flat base pass-through, so the visible background matches the
        // SkyLight gradient the metals reflect (an otherwise-incoherent scene: a metal shows
        // a bright sky the viewer never sees, floating against a dark void). A scene with NO
        // SKY entry in the light table keeps the byte-identical dark pass-through (the
        // 0%-gate: a scene without a SkyLight has no sky to render). No material fetch, no
        // normal/id decode — the light-table scan is the only extra work, and it runs ONLY
        // for background pixels (cheap: no geometry/lighting there).
        float3 ro_bg, rd_bg;
        generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro_bg, rd_bg);

        LightHeader H_bg = load_light_header(LightBuf);
        bool has_sky = false;
        float3 sky_color = float3(0.0, 0.0, 0.0);
        float3 ground_color = float3(0.0, 0.0, 0.0);
        float3 sun_disc = float3(0.0, 0.0, 0.0);
        for (uint bi = 0u; bi < H_bg.l0a_count; ++bi) {
            LightElem BL = load_light(LightBuf, bi);
            if (light_kind(BL) == LIGHT_KIND_SKY) {
                has_sky = true;
                sky_color = BL.color;      // upper hemisphere (L.color)
                ground_color = BL.pos;     // lower hemisphere (packed in the pos lane)
            } else if (light_kind(BL) == LIGHT_KIND_DIRECTIONAL) {
                // A visible sun disc for every directional light — the SAME `pow`-kernel
                // shape the metal's own sun-disc term uses (`sun_kernel`), but a FIXED
                // exponent (the background is a flat environment element, not a BRDF lobe).
                float3 bl = normalize(BL.dir);
                sun_disc += BL.color * pow(saturate(dot(rd_bg, bl)), SKY_SUN_EXPONENT);
            }
        }

        if (has_sky) {
            float3 sky = lerp(ground_color, sky_color, saturate(dot(rd_bg, LIGHT_UP) * 0.5 + 0.5));
            sky += sun_disc;
            // O3: exposure is the FINAL multiply on the linear radiance, THEN the SAME
            // tonemap + manual gamma OETF the lit path applies (`aces_fitted`'s doc comment).
            sky *= H_bg.exposure;
            sky = tonemap_select(sky, load_tonemap_mode(LightBuf));
            sky = pow(sky, OETF_GAMMA_EXP);
            lit = sky;
        } else {
            // No SkyLight in this scene's table — keep the dark pass-through byte-identical.
            lit = base;
        }
    }

#if SHADOW_STAGE == SHADOW_STAGE_VIS
    // Rung 3a VIS: the VIS stage produces NO lit output. By this convergent point EVERY pixel has
    // settled EXACTLY ONE gShadowVis texel — the mesh arm wrote `float2(mesh_vis, 1.0)` for a
    // primary-directional shadow-receiving pixel, and the top-of-main neutral seed
    // (`float2(1.0, 0.0)`) covers every other pixel (background, `NoL <= 0`, `csm_mode == OFF`,
    // non-directional, or `mask == 0`). RETURN before the sole `gLit` store so VIS never writes the
    // lit target. (`lit` above is computed but discarded — the dead lighting work does not affect
    // the VIS output; the à-trous pass + the RESOLVE_DENOISED stage consume `gShadowVis` alone.)
    return;
#endif

    gLit[uint2(px, py)] = float4(clamp(lit, 0.0, 1.0), 1.0);
}
