// R9 VB geo/shade split plan (docs/R9-VB-SPLIT-PLAN.md, rung R9b/R9c/R9d, Section 5): the
// split's LIT PRODUCER. RE-fetches + RE-interpolates + RE-shades every pixel from scratch
// (Decision 5's pure-VB re-derive model, unchanged by the split -- `vb_geo.comp.hlsl`'s
// `gThinNormal` is written for the SSAO/denoise gather ONLY, never read back here). The
// shading tail is CHARACTER-IDENTICAL to `vb_resolve.comp.hlsl`'s own (the `vb_shade.comp.hlsl`
// discipline: sentinel skip, ALL-LIGHTS loop, duplicated pure helpers, tonemap tail) PLUS three
// pre-light additions this file's header documents below. `vb_shade.comp.hlsl` (the VB-P2
// CLASSIFIED lit producer) is a SEPARATE, untouched file -- Ã‚Â§0's naming rule: `vb_shade` stays
// the fused-mode classification producer, `vb_shade_split` is this R9 pre-light-split producer.
//
// # Sentinel / miss handling
//
// `instance_id == VB_ID_SENTINEL` marks a pixel `vb_raster` never covered: this pass WRITES
// NOTHING for such a pixel, so whatever `vb_sky` already painted this frame stands (the SAME
// "misses write nothing" contract `vb_resolve.comp.hlsl`/`sdf_forward_march.comp.hlsl` document).
//
// # (a) SSAO -- the Filament AO decoupling (`deferred_pbr.hlsl:940-977` verbatim mechanism)
//
// `gSsao` (Set 1 binding 4, r8 STORAGE) is read ONLY inside the SAME `ssao_mode != SSAO_MODE_OFF`
// structural gate the deferred resolve uses (`load_ssao_mode(LightBuf)`, freeze-guarded upstream
// per docs/R9-VB-SPLIT-PLAN.md Ã‚Â§1.4 -- no new host gate here). VB has no analytic march AO
// (mesh-only, no SDF field): unlike the deferred resolve's `ao_class = (view_t>=1e30)?1.0:ao`
// mesh-XOR-SDF duality, every VB pixel already IS the mesh case, so `ao_final` starts at the
// SAME Deferred-mesh-pixel constant `1.0` `vb_resolve.comp.hlsl`'s own comment documents, then
// combines the SSAO gather's r8 lane when armed (`min(1.0, ssao_blurred)`). `spec_ao`
// (Filament SpecularAO_Lagarde) is the SAME formula, verbatim, already present in
// `vb_resolve.comp.hlsl`'s tail -- decoupling diffuse-AO from specular-AO needs no new math,
// only a non-constant `ao_final` input. `ssao_mode == 0` (every split-without-SSAO config: the
// R9c DDGI-only / R9d Temporal-only residuals) never reads `gSsao` -- byte-identical to the v1
// fused baseline (the 0%-gate).
//
// # (b) R9c DDGI -- the probe-irradiance injection (`deferred_pbr.hlsl`'s I3 mesh-pixel arm)
//
// `ddgi_resolve.hlsli`'s shared `ddgi_probe_sample` (`#include`d below, after the Set-1
// bindings it consumes are declared -- that header's INCLUDE CONTRACT) samples the octahedral
// probe atlas and ADDS `diffuse_color * gi * ao_final` to `ambient`, mirroring the deferred
// resolve's I3 injection site verbatim (same corner-iteration order, same `precise` op-order
// pin). Gated by the SAME `ddgi_mode != 0u` header-word test (`load_ddgi_mode(LightBuf)`,
// already declared in `light_table.hlsli`) -- the GI-off byte-identity discipline (the
// `58f6c6c3` pin) holds structurally: on the OFF path the 3 DDGI-atlas bindings (Set 1
// bindings 5/6/7/8/9) stay bound-but-unread and the injection block never runs. Reachable only
// VBÃƒâ€”Both this rung (`ddgi_update` arming already ANDs `sdf_leg`, docs/R9-VB-SPLIT-PLAN.md
// Ã‚Â§1.3); compiled in NOW but runtime-off until R9c arms the host-side `path_vb_ddgi()` gate.
//
// # (c) R9d HWRT -- the denoised mesh-shadow visibility (`deferred_pbr.hlsl`'s DENOISED arm)
//
// `#if HWRT`, `gShadowVis` (Set 1 binding 10, rg16 STORAGE) is the FINAL Ãƒ -trous-filtered
// mesh-shadow visibility the framegraph's OWN `tlas -> shadow_vis -> shadow_atrous ->
// shadow_temporal` chain produces BEFORE this pass runs (docs/R9-VB-SPLIT-PLAN.md Ã‚Â§3 pass
// order) -- this file never traces a ray itself, it only CONSUMES the chain's output, exactly
// as `deferred_pbr.hlsl`'s `SHADOW_STAGE_RESOLVE_DENOISED` arm does: a single `.Load` +
// `min`-combine into the primary directional's `vis`, REPLACING the CSM shadow-map sample for
// that light (the mesh-shadow term supersedes the raster CSM comparison, same predicate gate
// `csm_mode != CSM_MODE_OFF && NoL > 0.0`). NOT the RESOLVE_INLINE/VIS trace arms (this file
// has no TLAS/RayShadowUbo/spec-const -- those live in the separate `shadow_vis` producer pass).
//
// # Material (v1 scope cut -- base compile NON-TEXTURED; `#if TEXTURED` compiles a SECOND .spv)
//
// This ONE source compiles TWICE (base + `-D TEXTURED=1`), mirroring `vb_shade.comp.hlsl`'s
// OWN established `#ifdef TEXTURED` idiom (a single source, two `.spv` outputs) rather than a
// separately-authored `_tex` file -- the manifest single-source rule. Non-textured: `base_color`/
// `metallic`/`roughness`/`emissive` come from the `Materials` SSBO keyed by the per-instance
// material ring's `id` (`PerInstanceMaterial`). Textured: the bindless SampleGrad splice is a
// near-verbatim copy of `vb_shade.comp.hlsl`'s own TEXTURED block (albedo/normal/metal-rough/
// AO/emissive, `PerInstanceMaterialTex`, Set 3 `gTextures[]`+`gTexSampler`) -- see that file's
// header doc for the full TBN/ORM/AO-texture rationale, unchanged here.
//
// # Bindings
//
//   Set 0 (`vb_layout0`, REUSED verbatim -- all 8 bindings, the R2 contract):
//     b0/u0: StructuredBuffer<VbInstanceRow>      gVbInstances     (`vb_geom_fetch.hlsli`)
//     t1   : StructuredBuffer<PerInstanceMaterial> instance_materials (base) OR
//            StructuredBuffer<PerInstanceMaterialTex> instance_materials_tex (`-D TEXTURED`)
//     b2   : cbuffer Camera                                        80-byte extent/camera block
//     t3   : StructuredBuffer<uint>                LightBuf         Lighting L0 light table
//     t4   : StructuredBuffer<MaterialGpu>          Materials       PBR material table
//     t5   : Texture2D<uint2>                       gVbId            the `vb_id` raster output
//     u6   : RWTexture2D<float4> (rgba8)             gLit            WRITE, the lit target
//     u7   : RWByteAddressBuffer                     gClassify        bound-but-unread (split
//                                                                      displaces classification)
//   Set 1 (`vb_split_layout1`, NEW, 11 bindings -- a DISTINCT layout object so `forward_layout1`
//   stays byte-untouched by the Forward family / the fused `vb_resolve`):
//     @0 (t12/s12) : Texture2DArray<float>+SamplerComparisonState  gCsm/gCsmCmp    COMBINED,
//                                                                    `forward_layout1`'s shadow
//                                                                    table VERBATIM
//     @1 (b13)     : cbuffer CsmCascades                            VERBATIM
//     @2 (t14/s14) : Texture2DArray<float>+SamplerComparisonState  gShadowAtlas/Cmp COMBINED,
//                                                                    VERBATIM
//     @3 (b15)     : cbuffer ShadowAtlas                            VERBATIM
//     @4 (u11)     : RWTexture2D<float> (r8)                       gSsao            READ (a)
//     @5 (t16/s20) : Texture2DArray<float4>+SamplerState           gDdgiIrr/Samp    READ (b) --
//                                                                    COMBINED (same vk::binding,
//                                                                    the deferred t16/s16 idiom;
//                                                                    the RHI has no standalone
//                                                                    SAMPLER descriptor kind --
//                                                                    `ddgi_resolve.hlsli`'s taps
//                                                                    take tex+sampler params and
//                                                                    compose either way)
//     @6 (t17/s21) : Texture2DArray<float2>+SamplerState           gDdgiDepth/Samp  READ (b),
//                                                                    COMBINED (see @5)
//     @7 (b18)     : cbuffer ResolvedDdgi                          READ (b), SAME layout
//                                                                    `deferred_pbr.hlsl` declares
//     @8 (u19)     : RWTexture2D<float2> (rg16)  `#if HWRT` ONLY   gShadowVis       READ (c)
//   9 bindings total (0..8) <= 24 (8 on the software leg -- @8 is hwrt-declared only, its layout
//   entry cfg-gated host-side to keep the software set an exact fill). Set 2 (geometry,
//   UNCHANGED) -- `vb_geom_fetch.hlsli`'s own.
//   Set 3 (`#if TEXTURED` ONLY) -- the shared bindless texture-array table, the SAME layout
//   OBJECT `gbuffer_mrt.fs.hlsl`/`vb_shade_tex`'s TEXTURED variant binds.
//
// # Push constant (64 bytes -- UNCHANGED shape from `vb_resolve.comp.hlsl`'s own)
//
//   offset 0: float4x4 view_proj   the SAME reverse-Z proj*view `vb_raster.vs.hlsl` used
//
// # Arming
//
// Recorded iff `path_vb_split()` (docs/R9-VB-SPLIT-PLAN.md Ã‚Â§2), paired 1:1 with
// `vb_geo.comp.hlsl`; per-frame base/`_tex` selection mirrors the fused `vb_resolve`/
// `vb_shade_tex` pick (`vb_tex_active()`, boot-frozen split arming, per-frame base/_tex choice).
//
// Compiled offline (hermetic build -- no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_shade_split.comp.hlsl -Fo vb_shade_split.comp.spv
//   (TEXTURED variant: add `-D TEXTURED=1 -Fo vb_shade_split_tex.comp.spv`)
//   (R9d HWRT variant, NOT YET WIRED at the pipeline-build level this rung -- the `#if HWRT`
//   declarations above are authored now so the descriptor-set layout is ready. A plain `.Load`
//   + min-combine, no `rayQuery`, so the SAME `cs_6_0` target as the base compile suffices):
//   add `-T cs_6_0 -D HWRT=1 -Fo vb_shade_split_hwrt.comp.spv`
//   (HWRT + TEXTURED combo, same rationale): add
//   `-T cs_6_0 -D TEXTURED=1 -D HWRT=1 -Fo vb_shade_split_tex_hwrt.comp.spv`
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_shade_split.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_shade_split_tex.comp.spv

// binding 0 (`gVbInstances`, the `VbInstanceRow` SSBO) is declared inside `vb_geom_fetch.hlsli`
// itself (that file's own INCLUDE CONTRACT -- self-contained, needs nothing pre-declared).
#include "vb_pack.hlsli"
#include "vb_geom_fetch.hlsli"

#ifdef TEXTURED
// binding 1 (TEXTURED variant): the per-instance TEXTURED material payload -- byte-identical
// 48 B shape to `boyko_render::mesh_draw::PerInstanceMaterialTex` / `vb_shade.comp.hlsl`'s own
// declaration (this file's TEXTURED splice is a near-verbatim copy of that file's own).
struct PerInstanceMaterialTex {
    float4 base_color;
    uint   material_id;
    uint   albedo;
    uint   normal;
    uint   metal_rough;
    uint   ao;
    uint   emissive;
    float  metallic;
    float  roughness;
};
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterialTex> instance_materials_tex;
#else
// binding 1: the per-instance material payload -- byte-identical shape to
// `vb_resolve.comp.hlsl`'s own declaration.
struct PerInstanceMaterial {
    float4 base_color;
    uint   id;
    uint3  _pad;
};
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;
#endif

// binding 2: the extent/camera UNIFORM block -- byte-identical shape to every other consumer's.
[[vk::binding(2, 0)]] cbuffer Camera {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};

// binding 3: the Lighting-L0 light table (word-indexed `[LightHeaderGpu || GpuLight[]]`).
[[vk::binding(3, 0)]] StructuredBuffer<uint> LightBuf;

// binding 4: the material table -- byte-identical `MaterialGpu` shape to every other consumer.
struct MaterialGpu {
    float4 base_color;
    float4 mrr;
    float4 emissive;
};
[[vk::binding(4, 0)]] StructuredBuffer<MaterialGpu> Materials;

// binding 5: the `vb_id` raster output (R32G32_UINT) -- SAMPLED, unfiltered `.Load` fetch.
Texture2D<uint2> gVbId : register(t5);

// binding 6: the shared `lit` STORAGE image -- WRITE, this pass's own output.
[[vk::image_format("rgba8")]] RWTexture2D<float4> gLit : register(u6);

// binding 7: the packed classify buffer. Bound-but-unread -- split displaces classification
// (Ã‚Â§0), but the descriptor stays declared for the ONE shared `vb_layout0` layout object.
#include "vb_classify_common.hlsli"

#ifndef VB_SV0_KILL
// VB-SV0 DP2: the dedicated `sdf_mesh_shadow` pass's R8G8 term — a VERBATIM mirror of
// `vb_resolve.comp.hlsl`'s own binding-10 block (that block's doc carries the ~+75%/`13f1c9a3`
// argument for why the march is a PASS and this is two `.Load`s).
[[vk::binding(10, 0)]] Texture2D<float2> gSdfTerm : register(t10);
#endif

#ifdef TEXTURED
// Set 3 (`#if TEXTURED` ONLY): the shared bindless texture-array table -- the SAME layout
// OBJECT `vb_shade.comp.hlsl`'s own TEXTURED Set 3 binds. A runtime-sized `Texture2D[]`
// (binding 0) + the ONE shared immutable sampler (binding 1). Slot `0` is the reserved
// error-texture slot -- every real material slot is `!= 0`; every sample below is gated.
[[vk::binding(0, 3)]] Texture2D gTextures[] : register(t0, space3);
[[vk::binding(1, 3)]] SamplerState gTexSampler : register(s0, space3);
#endif

// --- Shading (cloned from `forward_opaque.fs.hlsl` via `vb_resolve.comp.hlsl` -- see that
// file's header doc) -----------------------------------------------------------------------

// `pbr_lighting.hlsli`'s INCLUDE CONTRACT precondition: `PI` + `LIGHT_UP` in scope first.
static const float PI = 3.14159265358979323846;
static const float3 LIGHT_UP = float3(0.0, 1.0, 0.0);

#include "pbr_lighting.hlsli"
#include "light_table.hlsli"

// --- Set 1 (`vb_split_layout1`, NEW): the shadow table (@0-3, VERBATIM from `forward_layout1`
// / `vb_resolve.comp.hlsl`'s own Set-1 block) + the R9b/R9c/R9d additions (@4-10). A DISTINCT
// layout object -- `forward_layout1` itself (and the fused `vb_resolve`/`vb_shade`) stay
// byte-untouched (this file's header doc).

static const uint MAX_CASCADES = 4u;
struct CascadeData {
    float4x4 view_proj;
    float    split_far;
    float    texel_size;
    float2   _pad;
};
[[vk::binding(0, 1)]] Texture2DArray<float> gCsm : register(t12);
[[vk::binding(0, 1)]] SamplerComparisonState gCsmCmp : register(s12);
[[vk::binding(1, 1)]] cbuffer CsmCascades {
    CascadeData gCascades[MAX_CASCADES];
    uint gCsmActive;
    uint gCsmMode;
    uint gCsmPcfKernel;   // rung E1: the CsmPcfKernel word â€” `csm_pcf_disc` branches on it
    uint _gCsmPad;
};

static const uint M_SLOTS = 16u;
struct FaceTransform {
    float4x4 view_proj;
    float3   light_pos;
    float    inv_range;
};
[[vk::binding(2, 1)]] Texture2DArray<float> gShadowAtlas : register(t14);
[[vk::binding(2, 1)]] SamplerComparisonState gShadowAtlasCmp : register(s14);
[[vk::binding(3, 1)]] cbuffer ShadowAtlas {
    FaceTransform gFaces[M_SLOTS];
    uint gAtlasActive;
    uint gAtlasMode;
    uint2 _gAtlasPad;
};

// @4: the Render-P7 SSAO term (this file's header doc (a)) -- an r8 STORAGE image, `.Load`-read
// ONLY inside the `ssao_mode != SSAO_MODE_OFF` structural gate below. `[[vk::image_format
// ("r8")]]` pins the `OpTypeImage` to `R8` (matches the deferred resolve's own `gSsao` pin).
[[vk::binding(4, 1)]] [[vk::image_format("r8")]] RWTexture2D<float> gSsao : register(u11);

// @5/@6: the DDGI probe-irradiance atlas (this file's header doc (b)) -- Texture2DArray +
// SEPARATE SamplerState (NOT a combined-image-sampler; see this file's header doc for why).
[[vk::binding(5, 1)]] Texture2DArray<float4> gDdgiIrr : register(t16);
[[vk::binding(5, 1)]] SamplerState gDdgiIrrSamp : register(s20);

// @7/@8: the DDGI probe depth-moment atlas -- Texture2DArray<float2> (mean + mean^2), matching
// `deferred_pbr.hlsl`'s exact type, + its own SEPARATE SamplerState.
[[vk::binding(6, 1)]] Texture2DArray<float2> gDdgiDepth : register(t17);
[[vk::binding(6, 1)]] SamplerState gDdgiDepthSamp : register(s21);

// @9: the DDGI grid UBO -- byte-mirrors `boyko_render::ResolvedDdgi` (48 B), the SAME layout
// `deferred_pbr.hlsl` declares at its own binding 18 (this Set's binding NUMBER differs; the
// FIELD layout must not, since the host uploads one `ResolvedDdgi` shape everywhere).
[[vk::binding(7, 1)]] cbuffer ResolvedDdgi : register(b18) {
    float4 gDdgiOrigin;      // grid origin (probe (0,0,0) min world corner); .w padding
    float4 gDdgiInvSpacDims; // .x = inv_spacing; .yzw = bit-cast u32 dims (x, y, z)
    uint   gDdgiMode;        // mirrors ResolvedDdgi.ddgi_mode_word (unused -- gated on the
                              // LightBuf header word below, the single source of truth)
    uint3  _gDdgiPad;        // pad to the 48-byte ResolvedDdgi stride
};

#if HWRT
// @10 (`#if HWRT` ONLY): the Ã -trous-FILTERED mesh-shadow visibility (this file's header doc
// (c)) -- RG: R = filtered mesh_vis, G = validity. Declared ENTIRELY under `#if HWRT` so the
// software `.spv` never references it (the byte-identity gate, `deferred_pbr.hlsl`'s own
// `SHADOW_STAGE`-gated `gShadowVis` precedent). `[[vk::image_format("rg16")]]` matches the
// producer chain's RG16 ping-pong rings.
[[vk::binding(8, 1)]] [[vk::image_format("rg16")]] RWTexture2D<float2> gShadowVis : register(u19);
#endif

// SDFDDGI I3 (b): the shared DDGI resolve sample (`ddgi_probe_sample`) -- ONE source of truth
// with `deferred_pbr.hlsl` / the GPU golden. Included AFTER the gDdgiIrr/gDdgiDepth/ResolvedDdgi
// binding decls above (the tap helpers read them) -- that header's INCLUDE CONTRACT. Ordered
// BEFORE `shadow_apply.hlsli` below, matching `deferred_pbr.hlsl`'s own include order verbatim.
#include "ddgi_resolve.hlsli"

#include "shadow_apply.hlsli"

// --- Duplicated pure helpers (VERBATIM copy of `forward_opaque.fs.hlsl`'s own span, via
// `vb_resolve.comp.hlsl` -- see that file's "Duplicated (not shared) pure helpers" doc) -----

float2 env_brdf_approx(float roughness, float NoV) {
    const float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
    const float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
    float4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * NoV)) * r.x + r.y;
    return float2(-1.04, 1.04) * a004 + r.zw;
}

static const float SUN_KERNEL_EXPONENT_MIN = 1.0;
static const float SUN_KERNEL_EXPONENT_MAX = 2048.0;

float sun_kernel_exponent(float alpha) {
    float n = 2.0 / max(alpha * alpha, 1e-6) - 2.0;
    return clamp(n, SUN_KERNEL_EXPONENT_MIN, SUN_KERNEL_EXPONENT_MAX);
}

float sun_kernel(float3 dir, float3 sun_dir, float alpha) {
    float c = saturate(dot(dir, sun_dir));
    return pow(c, sun_kernel_exponent(alpha));
}

static const float SUN_ENV_WEIGHT = 1.0;

// The 64-byte push constant -- the geometry-fetch reprojection matrix (UNCHANGED shape from
// `vb_resolve.comp.hlsl`'s own).
[[vk::push_constant]] struct PushConstants {
    float4x4 view_proj;
} pc;

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    uint w = img_w_raw;
    uint h = img_h_raw;
    if (idx >= w * h) {
        return;
    }
    uint px = idx % w;
    uint py = idx / w;

    // SAFETY (memory ordering, not unsafe): this is a plain read of a value the graph's derived
    // COLOR_ATTACHMENT_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL barrier (recorded at this pass) makes
    // visible after `vb_raster`'s color write -- no unsynchronized access.
    uint2 packed = gVbId.Load(int3((int)px, (int)py, 0));
    VbId id = vb_id_unpack(packed);
    if (id.instance_id == VB_ID_SENTINEL) {
        // Misses write NOTHING -- the sky color `vb_sky` already painted this frame stands (the
        // SAME ownership-gate contract `vb_resolve.comp.hlsl`/`sdf_forward_march.comp.hlsl`
        // document).
        return;
    }

    float2 pixel_xy = float2((float)px, (float)py) + 0.5; // pixel-CENTER, matching SV_Position
    float2 extent = float2((float)w, (float)h);
    VbGeomFetchResult geo = vb_geom_fetch(id.instance_id, id.raw_prim_id, pixel_xy, pc.view_proj, extent);

    float3 n = normalize(geo.world_normal);
    float3 P = geo.world_pos;

#ifdef TEXTURED
    PerInstanceMaterialTex pmt = instance_materials_tex[id.instance_id];
#else
    PerInstanceMaterial pm = instance_materials[id.instance_id];
#endif

#ifdef TEXTURED
    // TV0-style splice (near-verbatim copy of `vb_shade.comp.hlsl`'s own TEXTURED block -- see
    // that file's header doc for the full TBN/ORM/AO-texture rationale, unchanged here).
    MaterialGpu mt = Materials[pmt.material_id];

    float2 ddx_uv = float2(geo.uv_grad.x, geo.uv_grad.z);
    float2 ddy_uv = float2(geo.uv_grad.y, geo.uv_grad.w);

    float3 albedo_tex_rgb = float3(1.0, 1.0, 1.0);
    if (pmt.albedo != 0u) {
        albedo_tex_rgb = gTextures[NonUniformResourceIndex(pmt.albedo)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).rgb;
    }
    float3 base = (pmt.albedo != 0u) ? albedo_tex_rgb * pmt.base_color.rgb : pmt.base_color.rgb;

    if (pmt.normal != 0u) {
        float3 N = n;
        float3 T = normalize(geo.world_tangent - dot(geo.world_tangent, N) * N);
        float3 B = cross(N, T) * geo.tex_w;
        float3 packed_n = gTextures[NonUniformResourceIndex(pmt.normal)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).xyz;
        float3 n_ts = normalize(packed_n * 2.0 - 1.0);
        n = normalize(T * n_ts.x + B * n_ts.y + N * n_ts.z);
    }

    float metallic = pmt.metallic;
    float roughness = pmt.roughness;
    if (pmt.metal_rough != 0u) {
        float3 mr = gTextures[NonUniformResourceIndex(pmt.metal_rough)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).rgb;
        metallic = mr.b;
        roughness = mr.g;
    }
    roughness = clamp(roughness, 0.045, 1.0); // fp32 floor, mirrors the deferred resolve

    // AO: modulates the SSAO combine's own `ao_final` below (mirrors `deferred_pbr.hlsl`'s
    // `ao * pbr.b` combine -- here folded into the `ao_final` seed instead of a march `ao`).
    float ao_tex = 1.0;
    if (pmt.ao != 0u) {
        ao_tex = gTextures[NonUniformResourceIndex(pmt.ao)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).r;
    }

    float emissive_mask = 1.0;
    if (pmt.emissive != 0u) {
        float3 em = gTextures[NonUniformResourceIndex(pmt.emissive)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).rgb;
        emissive_mask = dot(em, float3(0.2126, 0.7152, 0.0722));
    }

    float reflectance = mt.mrr.z; // no texture channel carries reflectance yet (mirrors deferred)
    float3 emissive = mt.emissive.rgb * emissive_mask;
#else
    MaterialGpu m = Materials[pm.id];
#endif

    float3 v = normalize(cam_eye.xyz - P);
    float NoV = max(dot(n, v), 1e-4);

#ifndef TEXTURED
    float3 base = m.base_color.rgb;
    float metallic = m.mrr.x;
    float roughness = clamp(m.mrr.y, 0.045, 1.0); // fp32 floor, mirrors the deferred resolve
    float reflectance = m.mrr.z;
    float3 emissive = m.emissive.rgb;
#endif
    float a = roughness * roughness; // GGX alpha = perceptual^2

    float3 f0 = lerp(0.16 * reflectance * reflectance, base, metallic);
    float3 diffuse_color = base * (1.0 - metallic);

    float2 dfg_v = env_brdf_approx(roughness, NoV);
    float Ess = max(dfg_v.x + dfg_v.y, 1e-4);
    float3 energy_comp = 1.0 + f0 * (1.0 / Ess - 1.0);

    Surface surf;
    surf.n = n;
    surf.NoV = NoV;
    surf.a = a;
    surf.f0 = f0;
    surf.diffuse_color = diffuse_color;
    surf.energy_comp = energy_comp;

    float3 R = reflect(-v, n);
    float hemi = dot(n, LIGHT_UP) * 0.5 + 0.5;

    // (a) R9b SSAO combine (this file's header doc) -- `ao_final` starts at the Deferred-mesh-
    // pixel constant `1.0` (VB is mesh-only, no analytic march AO to `min` against), THEN the
    // SSAO gather's r8 lane combines in when `ssao_mode != SSAO_MODE_OFF`. Under `#if TEXTURED`
    // an AO texture further modulates the seed BEFORE the SSAO combine (mirrors
    // `deferred_pbr.hlsl`'s `ao * pbr.b` ordering: texture AO first, SSAO `min`-combine after).
#ifdef TEXTURED
    float ao_final = ao_tex;
#else
    float ao_final = 1.0;
#endif
    uint ssao_mode = load_ssao_mode(LightBuf);
    if (ssao_mode != SSAO_MODE_OFF) {
        float ssao_blurred = gSsao.Load(int2(px, py)).r;
        ao_final = min(ao_final, ssao_blurred);
    }
#ifndef VB_SV0_KILL
    // VB-SV0 DP2: the 2-bit runtime gate, hoisted ONCE per pixel — a VERBATIM mirror of
    // `vb_resolve.comp.hlsl`'s own block (per-bit independence and the mode-0 identity argument
    // live there). Placed AFTER the SSAO combine to keep the split's stated AO ordering idiom
    // ("texture AO first, SSAO `min`-combine after", now SV0 after that) — `min` is exact and
    // order-independent, so the placement is readability, not arithmetic.
    uint sv0_mode = load_vb_sdf_mesh_mode(LightBuf);
    float2 sv0_term = float2(1.0, 1.0);
    if (sv0_mode != 0u) {
        sv0_term = gSdfTerm.Load(int3((int)px, (int)py, 0));
    }
    if ((sv0_mode & VB_SDF_MESH_AO_BIT) != 0u) {
        ao_final = min(ao_final, sv0_term.g);
    }
#endif
    float spec_ao = saturate(pow(NoV + ao_final, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao_final);

    LightHeader H = load_light_header(LightBuf);
    uint csm_mode = load_csm_mode(LightBuf);
    uint punctual_shadow_mode = load_punctual_shadow_mode(LightBuf);
    uint ddgi_mode = load_ddgi_mode(LightBuf);

    float3 lit_direct = float3(0.0, 0.0, 0.0);
    float3 ambient = float3(0.0, 0.0, 0.0);
    bool primary_dir_seen = false;

    // L0a: directionals + sky. ALL-LIGHTS -- no cluster/froxel lookup (VB v1 is fused-only,
    // mirrors plain `Forward`'s own base compile).
    for (uint i = 0u; i < H.l0a_count; ++i) {
        LightElem L = load_light(LightBuf, i);
        if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
            float3 l = normalize(L.dir);
            float NoL = max(dot(n, l), 0.0);
            // v1 mesh-only: no baked SDF shadow term -- `vis` starts at the Deferred-mesh-pixel
            // constant `1.0`, then CSM (or, under `#if HWRT`, the denoised mesh trace) min-
            // combines in.
            float vis = 1.0;
            if (!primary_dir_seen) {
                primary_dir_seen = true;
                if (csm_mode != CSM_MODE_OFF && NoL > 0.0) {
                    float view_z = dot(cam_forward.xyz, P - cam_eye.xyz);
#if HWRT
                    // (c) R9d: the denoised mesh-shadow visibility REPLACES the CSM shadow-map
                    // sample for the primary directional (this file's header doc (c) --
                    // `deferred_pbr.hlsl`'s `SHADOW_STAGE_RESOLVE_DENOISED` arm, verbatim
                    // min-combine, same predicate gate).
                    float mesh_vis = gShadowVis.Load(int2(px, py)).r;
                    vis = min(vis, mesh_vis);
#else
                    vis = min(vis, csm_visibility(P, n, view_z, NoL));
#endif
                }
#ifndef VB_SV0_KILL
                // VB-SV0 DP2: the shadow half, beside the CSM/denoised combine and OUTSIDE its
                // `if` — a VERBATIM mirror of `vb_resolve.comp.hlsl`'s own block (the
                // independence and by-construction pairing arguments live there). Composes with
                // the HWRT denoised arm the same way it composes with CSM: one more `min` source.
                if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u) {
                    vis = min(vis, sv0_term.r);
                }
#endif
            }
            PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
            lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * vis) * L.color;

            float sun_k = sun_kernel(R, l, a);
            float3 sun_spec_ambient = eval_pbr_sun_disc(surf, dfg_v, sun_k, L.color) * SUN_ENV_WEIGHT;
            ambient += sun_spec_ambient * spec_ao;
        } else if (light_kind(L) == LIGHT_KIND_SKY) {
            float3 sky_color = L.color;
            float3 ground_color = L.pos;
            ambient += eval_pbr_ambient_hemi(surf, R, dfg_v, sky_color, ground_color, hemi, ao_final, spec_ao);
        }
    }

    // (b) R9c DDGI probe-irradiance injection (this file's header doc) -- ADDED to `ambient`,
    // mirroring `deferred_pbr.hlsl`'s I3 mesh-pixel arm verbatim (same corner order, same
    // `precise` op-order pin inside `ddgi_probe_sample`). 0%-gate: on `ddgi_mode == 0` (every
    // pre-R9c split scene) this block never runs, so the DDGI bindings stay bound-but-unread
    // and the lit pixels are byte-identical.
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

    // L0b: the point/spot block. ALL-LIGHTS flat scan, TOKEN-FOR-TOKEN clone of
    // `forward_opaque.fs.hlsl`'s own non-FROXEL arm.
    for (uint j = H.l0a_count; j < H.light_count; ++j) {
        LightElem L = load_light(LightBuf, j);
        float3 toL = L.pos - P;
        float d2 = dot(toL, toL);
        float range2 = L.range * L.range;
        if (d2 > range2) {
            continue;
        }
        float inv_d = rsqrt(max(d2, 1e-8));
        float3 l = toL * inv_d;
        float win = saturate(1.0 - (d2 * d2) / (range2 * range2));
        float atten = (1.0 / max(d2, 1e-4)) * win * win;
        if (light_kind(L) == LIGHT_KIND_SPOT) {
            float2 cones = unpack_cones(L.cone_pack);
            float3 spot_dir = safe_normalize(L.dir);
            float cosA = dot(-l, spot_dir);
            float denom = max(cones.x - cones.y, 1e-4);
            float tt = saturate((cosA - cones.y) / denom);
            atten *= tt * tt;
        }

        float punctual_shadow = 1.0;
        if (punctual_shadow_mode != PUNCTUAL_SHADOW_MODE_OFF) {
            uint slot = light_atlas_slot(L.kind);
            if (slot != SLOT_NONE) {
                float pnol = max(dot(n, l), 0.0);
                if (light_kind(L) == LIGHT_KIND_SPOT) {
                    punctual_shadow = spot_atlas_visibility(slot, P, n, pnol);
                } else if (light_kind(L) == LIGHT_KIND_POINT) {
                    punctual_shadow = punctual_atlas_visibility(slot, P, n, pnol);
                }
            }
        }

        float NoL = max(dot(n, l), 0.0);
        PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
        lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * punctual_shadow) * atten * L.color;
    }

    // Same tail as `forward_opaque.fs.hlsl` / the deferred resolve: exposure LAST, THEN the
    // owner-selected tonemap, THEN the manual gamma-2.2 OETF.
    float3 lit = (lit_direct + ambient + emissive) * H.exposure;
    lit = tonemap_select(lit, load_tonemap_mode(LightBuf));
    lit = pow(lit, OETF_GAMMA_EXP);
    gLit[uint2(px, py)] = float4(clamp(lit, 0.0, 1.0), 1.0);
}
