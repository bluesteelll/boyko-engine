// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), authored rung P2a, wired as a
// selectable `lit` producer at rung P2c (`Renderer::declare_vb_graph`/`record_vb`, host-selected
// via `GBufferScene::vb_use_classified`). `vb_shade` -- the material-classified shading dispatch
// ("The pipeline" step 5):
// `vkCmdDispatch(G + present_material_count, 1, 1)` (a REGULAR dispatch, NOT
// `vkCmdDispatchIndirect` -- the FFI lacks indirect dispatch, plan D2). Each group `g`:
// `mat=group_to_mat[g]`; SENTINEL -> return; `slot=(g-gbase[mat])*64+(tid&63)`;
// `slot>=counts[mat]` -> return; `idx=pixel_list[offsets[mat]+slot]`; `px=idx%w; py=idx/w`;
// then the SAME shading tail `vb_resolve.comp.hlsl` runs.
//
// # Byte-identity by construction (plan D3)
//
// This file is `vb_resolve.comp.hlsl` PLUS an ~8-line pixel-selection prologue swap: the
// tid-linear `idx=tid.x; px=idx%w; py=idx/w` computation becomes the classify-table lookup
// above (`idx=cls_pixel(...)`; `px=idx%w; py=idx/w` -- the SAME two instructions,
// `vb_resolve.comp.hlsl:214-215`). Every line from the `gVbId.Load` re-fetch onward (the
// sentinel re-check, `vb_geom_fetch`, the PBR + `shadow_apply` shading, the tonemap tail,
// `gLit[uint2(px,py)]=...`) is CHARACTER-IDENTICAL to `vb_resolve.comp.hlsl`'s own -- DO NOT
// edit that span without also re-verifying `vb_resolve.comp.hlsl` line-for-line. Per-pixel
// shading is independent of HOW a pixel's `(px,py)` was selected, so regrouping cannot change
// any pixel's output bytes: for today's flat (non-textured) materials, this pass (forced via
// `BOYKO_VB_FORCE_CLASSIFIED=1`, rung P2c) must reproduce the SAME
// `vb_mesh`/`vb_both`/`vb_sdf_only` goldens the fused `vb_resolve` produces -- a moved hash is a
// bug, not a re-pin.
//
// # Sentinel / miss handling
//
// The re-checked `instance_id == VB_ID_SENTINEL` branch is DEAD by construction (the classify
// `scatter` pass only ever writes non-sentinel pixels into `pixel_list`) but kept for
// byte-identity with `vb_resolve.comp.hlsl`'s own defensive check.
//
// # Shading (cloned from `forward_opaque.fs.hlsl`, token-for-token where noted -- see
// `vb_resolve.comp.hlsl`'s own header doc for the full rationale, unchanged here)
//
// # Material (base compile -- NON-TEXTURED ONLY)
//
// TV0 (`RENDER-PARITY-PLAN.md` §2.3 / `docs/VB-P2-CLASSIFICATION-PLAN.md`'s "Open items"):
// compiled with `-D TEXTURED=1` (a SEPARATE `.spv`, `vb_shade_tex.comp.spv`; the base
// `vb_shade.comp.spv` stays byte-frozen), the whole-lights loop below keys the bindless
// texture fetch off THIS GROUP's uniform `mat = group_to_mat[gid.x]` invariant (P2b's
// debug-assert proves `mat == instance_materials[id.instance_id].id` per group) -- every
// thread in the group samples `gTextures[]` at effectively the SAME index, the entire payoff
// of landing textures on the classified pipeline instead of the fused `vb_resolve`
// (`NonUniformResourceIndex` is kept as a correctness belt since the compiler cannot prove
// the invariant statically, not because the index is genuinely divergent).
//
// # Bindings
//
//   Set 0 (VB core + images + classify -- `vb_layout0`; NOT `forward_layout0`):
//     b0/u0: StructuredBuffer<VbInstanceRow>      gVbInstances    (`vb_geom_fetch.hlsli`)
//     t1   : StructuredBuffer<PerInstanceMaterial> instance_materials (base) OR
//            StructuredBuffer<PerInstanceMaterialTex> instance_materials_tex (`-D TEXTURED`,
//            binds the WIDER 48 B ring -- a distinct descriptor SET instance against the SAME
//            `vb_layout0` layout OBJECT, since Vulkan's `STORAGE_BUFFER` binding shape does not
//            encode the bound buffer's element stride)
//     b2   : cbuffer Camera                                       80-byte extent/camera block
//     t3   : StructuredBuffer<uint>                LightBuf        Lighting L0 light table
//     t4   : StructuredBuffer<MaterialGpu>          Materials      PBR material table
//     t5   : Texture2D<uint2>                       gVbId           the `vb_id` R32G32_UINT
//                                                                    raster output (SAMPLED,
//                                                                    `.Load` unfiltered fetch)
//     u6   : RWTexture2D<float4> (rgba8)             gLit           the shared `lit` target
//     u7   : RWByteAddressBuffer                     gClassify       the packed classify
//                                                                    buffer (via
//                                                                    `vb_classify_common.hlsli`)
//   Set 1 (shadow) -- VERBATIM copy of `vb_resolve.comp.hlsl`'s own Set-1 block (see that
//   file's header doc).
//   Set 2 (geometry) -- `vb_geom_fetch.hlsli`'s own `gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta`.
//   Set 3 (`-D TEXTURED` ONLY) -- the shared bindless texture-array table, the SAME layout
//   OBJECT `gbuffer_mrt.fs.hlsl`'s TEXTURED variant binds (R5 -- one shared layout, never a
//   structurally-identical-but-distinct handle): `gTextures[]` @0 + `gTexSampler` @1.
//
// # Push constant (64 bytes -- the geometry-fetch reprojection matrix, UNCHANGED shape from
// `vb_resolve.comp.hlsl`'s own -- no `material_count` field: this file's prologue derives
// every `gClassify` offset from `w`/`h` alone, see `vb_classify_common.hlsli`'s sync-pin doc)
//
//   offset 0: float4x4 view_proj   the SAME reverse-Z proj*view `vb_raster.vs.hlsl` used
//
// Compiled offline (hermetic build -- no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_shade.comp.hlsl -Fo vb_shade.comp.spv
//   (TEXTURED variant: add `-D TEXTURED=1 -Fo vb_shade_tex.comp.spv`)
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_shade.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_shade_tex.comp.spv

// binding 0 (`gVbInstances`, the `VbInstanceRow` SSBO) is declared inside `vb_geom_fetch.hlsli`
// itself (that file's own INCLUDE CONTRACT -- self-contained, needs nothing pre-declared).
#include "vb_pack.hlsli"
#include "vb_geom_fetch.hlsli"

#ifdef TEXTURED
// binding 1 (TEXTURED variant): the per-instance TEXTURED material payload -- byte-identical
// 48 B shape to `boyko_render::mesh_draw::PerInstanceMaterialTex` / `gbuffer_mrt.vs.hlsl`'s own
// declaration. A DEDICATED binding-1 buffer (a wider ring than the base `PerInstanceMaterial`)
// -- the host binds a DISTINCT descriptor SET against the SAME `vb_layout0` layout OBJECT for a
// textured frame (Vulkan's `STORAGE_BUFFER` binding shape carries no element-stride
// constraint), never a second layout.
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

// binding 2: the extent/camera UNIFORM block -- byte-identical shape to
// `vb_resolve.comp.hlsl`'s own declaration.
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

// binding 6: the shared `lit` STORAGE image.
[[vk::image_format("rgba8")]] RWTexture2D<float4> gLit : register(u6);

// binding 7: the packed classify buffer -- P2a NEW binding (`vb_classify_common.hlsli`'s own
// `[[vk::binding(7, 0)]]` declaration). Included AFTER `Camera` is declared above so
// `cls_pixel`/`cls_group_count`'s explicit `w`/`h` PARAMETERS (not an ambient global read --
// see that file's INCLUDE CONTRACT doc) can be fed `img_w_raw`/`img_h_raw` from this file's
// `main()` below.
#include "vb_classify_common.hlsli"

#ifdef TEXTURED
// Set 3 (TEXTURED variant ONLY, TV0): the shared bindless texture-array table -- the SAME
// layout OBJECT `gbuffer_mrt.fs.hlsl`'s own TEXTURED Set 1 binds (R5, `bindless.set()
// .set_layout()`), just a different Vulkan set INDEX here (Set 3 is the Vulkan-guaranteed
// floor for this 4-set pipeline -- `vb_geom_fetch.hlsli`'s own doc has the Set-numbering
// precedent). A runtime-sized `Texture2D[]` (binding 0) + the ONE shared immutable sampler
// (binding 1). Slot `0` is the reserved T4 error-texture slot -- every real material slot is
// `!= 0`; every sample below is gated `slot != 0`.
[[vk::binding(0, 3)]] Texture2D gTextures[] : register(t0, space3);
[[vk::binding(1, 3)]] SamplerState gTexSampler : register(s0, space3);
#endif

// --- Shading (cloned from `forward_opaque.fs.hlsl` -- see `vb_resolve.comp.hlsl`'s header
// doc) -----------------------------------------------------------------------------------

// `pbr_lighting.hlsli`'s INCLUDE CONTRACT precondition: `PI` + `LIGHT_UP` in scope first.
static const float PI = 3.14159265358979323846;
static const float3 LIGHT_UP = float3(0.0, 1.0, 0.0);

#include "pbr_lighting.hlsli"
#include "light_table.hlsli"

// Multi-paradigm render-path plan, rungs VB-P1a (the seam) + VB-P1b (armed) + VB-P1c (this
// classified variant): the froxel cluster-grid pair, Set 0 bindings 8/9 -- compiled in ONLY for
// the `-D FROXEL` variant (`vb_shade_froxel.comp.spv`, and `vb_shade_tex_froxel.comp.spv` with
// `-D TEXTURED=1`); the base (non-FROXEL, this file's default) compile never
// declares them, so its Set 0 stays byte-identical at 8 bindings and `vb_shade.comp.spv` stays
// byte-identical to its pre-VB-P1a build. The arm is LIVE in production since VB-P1b: the host
// selects this variant's pipeline + the 10-binding `vb_layout0_froxel` whenever
// `ResolvedRenderPath::froxel_light_cull` resolves true (the VB path AND
// `LightingConfig::clusters_enabled`, which DEFAULTS OFF -- an owner opt-in, NOT a structural
// disable, so this block is reachable code). Byte-identical shape to `forward_opaque.fs.hlsl`'s own
// `ClusterGrid`/`LightIndexList` (bindings 5/6 there) and `deferred_pbr.hlsl`'s (bindings 8/9
// there) -- the L1 cluster-cull pass (`cluster_cull.hlsl`) writes both, reused verbatim; the
// lookup helpers (`load_cluster_params`/`cluster_xy_tile`/`cluster_z_slice`/
// `cluster_linear_index`) are `light_table.hlsli`'s shared L1 helpers, already `#include`d above.
#ifdef FROXEL
[[vk::binding(8, 0)]] StructuredBuffer<uint2> ClusterGrid;     // {ps_offset, ps_count} per froxel
[[vk::binding(9, 0)]] StructuredBuffer<uint>  LightIndexList;  // flat surviving-index slices
#endif

#ifndef VB_SV0_KILL
// VB-SV0 DP2: the dedicated `sdf_mesh_shadow` pass's R8G8 term — a VERBATIM mirror of
// `vb_resolve.comp.hlsl`'s own binding-10 block (that block's doc carries the ~+75%/`13f1c9a3`
// argument for why the march is a PASS and this is two `.Load`s).
[[vk::binding(10, 0)]] Texture2D<float2> gSdfTerm : register(t10);
#endif

// --- Set 1 (shadow): a VERBATIM copy of `vb_resolve.comp.hlsl`'s own Set-1 block, so the SAME
// physical descriptor set (`ForwardTargets::set1`) binds to both.

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
    uint gCsmPcfKernel;   // rung E1: the CsmPcfKernel word — `csm_pcf_disc` branches on it
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

// The 64-byte push constant -- the geometry-fetch reprojection matrix (see this file's header
// doc: UNCHANGED shape from `vb_resolve.comp.hlsl`'s own).
[[vk::push_constant]] struct PushConstants {
    float4x4 view_proj;
} pc;

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID, uint3 gid : SV_GroupID) {
    // --- Classify-table pixel selection (P2a's ~8-line prologue swap, plan D3) -- replaces
    // `vb_resolve.comp.hlsl`'s tid-linear `idx=tid.x; px=idx%w; py=idx/w` (lines 208-215) with
    // the material-classified group -> pixel lookup. `w`/`h` stay local -- the tail below reads
    // them exactly as `vb_resolve.comp.hlsl`'s own tail does (its line 229's `extent`).
    uint w = img_w_raw;
    uint h = img_h_raw;
    uint mat = cls_g2m(gid.x);
    if (mat == VB_GROUP_SENTINEL) {
        return;
    }
    // Named `sel_slot` (not `slot`) so this prologue-only local does not shadow the shading
    // tail's OWN `uint slot = light_atlas_slot(...)` (the punctual-shadow block below) --
    // that tail is character-identical to `vb_resolve.comp.hlsl`'s own, unrenamed.
    uint sel_slot = (gid.x - cls_gbase(mat)) * 64u + (tid.x & 63u);
    if (sel_slot >= cls_count(mat)) {
        return;
    }
    uint idx = cls_pixel(cls_offset(mat) + sel_slot, w, h);
    uint px = idx % w;
    uint py = idx / w;

    // SAFETY (memory ordering, not unsafe): this is a plain read of a value the graph's derived
    // COLOR_ATTACHMENT_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL barrier (recorded at this pass) makes
    // visible after `vb_raster`'s color write -- no unsynchronized access.
    uint2 packed = gVbId.Load(int3((int)px, (int)py, 0));
    VbId id = vb_id_unpack(packed);
    if (id.instance_id == VB_ID_SENTINEL) {
        // Misses write NOTHING -- the sky color `vb_sky` already painted this frame stands (the
        // SAME ownership-gate contract `sdf_forward_march.comp.hlsl` documents). Dead by
        // construction here (see this file's header doc), kept for byte-identity.
        return;
    }

    float2 pixel_xy = float2((float)px, (float)py) + 0.5; // pixel-CENTER, matching SV_Position
    float2 extent = float2((float)w, (float)h);
    VbGeomFetchResult geo = vb_geom_fetch(id.instance_id, id.raw_prim_id, pixel_xy, pc.view_proj, extent);

    float3 n = normalize(geo.world_normal);
    float3 P = geo.world_pos;

    // VB-P2 classification plan, rung P2b note (now DISPATCHED as of rung P2c): the
    // classify-table's `mat` (this group's uniform material id, `cls_g2m(gid.x)` above) and
    // `pm.id` (this PIXEL's own per-instance material id, read here) are the SAME value by
    // construction (`vb_classify_count.comp.hlsl`/`vb_classify_scatter.comp.hlsl` both bin
    // `instance_materials[id.instance_id].id` == `mat` into this exact group). This is the
    // uniformity invariant TV0's bindless texture index (keyed off the GROUP's `mat`, not the
    // pixel's `pm.id`) relies on -- masked by byte-identity this rung (flat materials shade
    // identically either way). HLSL has no runtime assert facility, so the invariant stays
    // DOCUMENTED rather than checked in-shader by default (the debug guard below is the cheap
    // in-shader alternative, opt-in only) -- P2c's forced-classified golden re-run is the actual
    // verification (see the plan's "Open items").
#ifdef TEXTURED
    PerInstanceMaterialTex pmt = instance_materials_tex[id.instance_id];
#else
    PerInstanceMaterial pm = instance_materials[id.instance_id];
#endif

#ifdef VB_SHADE_DEBUG
    // Diagnostic-only invariant guard (never compiled into the shipped `vb_shade.comp.spv` --
    // no `-D VB_SHADE_DEBUG` in this file's header doc's dxc invocation, so this block is DEAD
    // in every frozen build and the output stays byte-identical with it absent). GPU asserts
    // aren't available and validation is off on this box, so a violation PAINTS solid magenta
    // instead of shading normally, making a classify-scatter bug visually obvious in a debug
    // recompile rather than silently mis-shading.
    if (pm.id != mat) {
        gLit[uint2(px, py)] = float4(1.0, 0.0, 1.0, 1.0);
        return;
    }
#endif

#ifdef TEXTURED
    // TV0 (`RENDER-PARITY-PLAN.md` §2.3): a near-verbatim splice of `gbuffer_mrt.fs.hlsl`'s
    // TEXTURED block (lines 223-320), retargeted to feed THIS pass's `base`/`metallic`/
    // `roughness`/`n`/`emissive`/`reflectance` locals directly (no G-buffer MRT intermediate --
    // Decision 5's pure-VB re-fetch-and-shade-from-scratch model) instead of writing them to
    // `gAlbedo`/`gNormal`/`gPbr`. The FINAL combine (AO/emissive texture MODULATING the
    // material's own scalar, metallic/roughness texture OVERRIDING the fallback) mirrors
    // `deferred_pbr.hlsl`'s own `MATERIAL_FLAG_TEXTURED_BIT` arm (its `gPbr` consumer) exactly,
    // since this pass has no separate resolve stage to apply that combine downstream.
    MaterialGpu mt = Materials[pmt.material_id];

    float2 ddx_uv = float2(geo.uv_grad.x, geo.uv_grad.z);
    float2 ddy_uv = float2(geo.uv_grad.y, geo.uv_grad.w);

    // Albedo: sampled (sRGB view -> hw-linear on sample) modulated by base_color, or
    // base_color alone when no albedo texture is bound. Slot `0` is the reserved T4
    // error-texture slot, never a real material's texture -- gated `!= 0`.
    float3 albedo_tex_rgb = float3(1.0, 1.0, 1.0);
    if (pmt.albedo != 0u) {
        albedo_tex_rgb = gTextures[NonUniformResourceIndex(pmt.albedo)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).rgb;
    }
    float3 base = (pmt.albedo != 0u) ? albedo_tex_rgb * pmt.base_color.rgb : pmt.base_color.rgb;

    // Tangent-space normal mapping: renormalize the interpolated geometric normal FIRST,
    // Gram-Schmidt the interpolated tangent against it, derive the bitangent via the
    // glTF/Mikktspace handedness sign (`tex_w` multiplies the BITANGENT), sample + unpack +
    // renormalize the tangent-space normal, then rotate it into world space via the TBN basis.
    // `normal == 0` keeps the geometric normal unperturbed. NO green-channel negation: the
    // engine's CANONICAL normal convention is DIRECTX (green-down, Unreal-style) and OpenGL
    // source packs are converted ONCE AT LOAD (`boyko_render::texture`'s `load_slot`), so the
    // map sampled here is already canonical (owner-set 2026-07-16; see `gbuffer_mrt.fs.hlsl`'s
    // GREEN-CHANNEL CONVENTION block for the full derivation), NOT glTF/Mikktspace.
    if (pmt.normal != 0u) {
        float3 N = n;
        float3 T = normalize(geo.world_tangent - dot(geo.world_tangent, N) * N);
        float3 B = cross(N, T) * geo.tex_w;
        float3 packed_n = gTextures[NonUniformResourceIndex(pmt.normal)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).xyz;
        float3 n_ts = normalize(packed_n * 2.0 - 1.0);
        n = normalize(T * n_ts.x + B * n_ts.y + N * n_ts.z);
    }

    // Metallic/roughness: glTF ORM channel convention (metallic = B, roughness = G) when the
    // metal-rough slot is bound, else the material's scalar fallback (`gPbr`'s "unbound ->
    // scalar fallback" contract).
    float metallic = pmt.metallic;
    float roughness = pmt.roughness;
    if (pmt.metal_rough != 0u) {
        float3 mr = gTextures[NonUniformResourceIndex(pmt.metal_rough)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).rgb;
        metallic = mr.b;
        roughness = mr.g;
    }
    roughness = clamp(roughness, 0.045, 1.0); // fp32 floor, mirrors the deferred resolve

    // AO: modulates the Deferred-mesh-pixel baseline (this pass's `ao_final` below), mirroring
    // `deferred_pbr.hlsl`'s `ao * pbr.b` combine.
    float ao_tex = 1.0;
    if (pmt.ao != 0u) {
        ao_tex = gTextures[NonUniformResourceIndex(pmt.ao)].SampleGrad(gTexSampler, geo.uv, ddx_uv, ddy_uv).r;
    }

    // Emissive: a luminance MASK (Rec.709 weights) modulating the material's own emissive
    // color, mirroring `deferred_pbr.hlsl`'s `emissive * pbr.a` combine.
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

    // v1 scope cut (mirrors `forward_opaque.fs.hlsl`'s own note): no SSAO/DDGI consumer is ever
    // armed under VB v1 (`cap_vb_v1_consumers`) -- `ao_final` stays the Deferred-mesh-pixel
    // constant `1.0`, TIMES the AO-texture mask under TEXTURED (mirrors `deferred_pbr.hlsl`'s
    // `ao * pbr.b` combine -- no separate SSAO term either way).
#ifdef TEXTURED
    float ao_final = ao_tex;
#else
    float ao_final = 1.0;
#endif
#ifndef VB_SV0_KILL
    // VB-SV0 DP2: the 2-bit runtime gate, hoisted ONCE per pixel — a VERBATIM mirror of
    // `vb_resolve.comp.hlsl`'s own block (per-bit independence and the mode-0 identity argument
    // live there). Under TEXTURED the AO half `min`-combines WITH the material's `ao_tex`, which
    // is exactly how the two AO sources compose everywhere else (`min` is exact and
    // order-independent on floats).
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

    float3 lit_direct = float3(0.0, 0.0, 0.0);
    float3 ambient = float3(0.0, 0.0, 0.0);
    bool primary_dir_seen = false;

    // L0a: directionals + sky. ALL-LIGHTS -- no cluster/froxel lookup (VB v1 is fused-only,
    // mirrors plain `Forward`'s own base compile). #ifdef FROXEL seam (a later rung, VB-P1):
    // this loop would gate on the cluster/light-index buffers instead of the flat L0a/L0b scan
    // below.
    for (uint i = 0u; i < H.l0a_count; ++i) {
        LightElem L = load_light(LightBuf, i);
        if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
            float3 l = normalize(L.dir);
            float NoL = max(dot(n, l), 0.0);
            // v1 mesh-only: no baked SDF shadow term -- `vis` starts at the Deferred-mesh-pixel
            // constant `1.0`, then CSM min-combines in.
            float vis = 1.0;
            if (!primary_dir_seen) {
                primary_dir_seen = true;
                if (csm_mode != CSM_MODE_OFF && NoL > 0.0) {
                    float view_z = dot(cam_forward.xyz, P - cam_eye.xyz);
                    vis = min(vis, csm_visibility(P, n, view_z, NoL));
                }
#ifndef VB_SV0_KILL
                // VB-SV0 DP2: the shadow half, beside the CSM combine and OUTSIDE its `if` — a
                // VERBATIM mirror of `vb_resolve.comp.hlsl`'s own block (the independence and
                // by-construction pairing arguments live there).
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

    // L0b: the point/spot block. #ifdef FROXEL (rung VB-P1a): the froxel-culled cluster walk,
    // gated on `use_clusters` -- the header's `clusters_enabled` bit AND, since VB-P1k, nonzero
    // dims AND the descriptor-derived capacity bound (all three built below) -- an armed frame
    // maps this pixel to its froxel and walks ONLY the survivors `cluster_cull.hlsl` wrote into
    // `ClusterGrid`/`LightIndexList` (the SAME lookup `forward_opaque.fs.hlsl`'s own FROXEL arm
    // performs); an unarmed frame (or the base, non-FROXEL compile) falls back to the IDENTICAL
    // flat `[l0a_count, light_count)` scan, TOKEN-FOR-TOKEN the SAME clone of
    // `forward_opaque.fs.hlsl`'s own non-FROXEL arm this file always ran. Note WHICH frames those
    // are on this path: unlike the Deferred/ForwardPlus readers, this compile never guards a
    // PLACEHOLDER buffer. `record_vb` binds it only under `scene.cluster_cull.is_some()`, and the
    // Set-0 it binds (`GBufferTargets::vb_set0_froxel`) is built only when the REAL
    // `cluster_grid`/`light_index` exist -- no `unwrap_or(light_table)` fallback -- so an unarmed
    // VB boot runs the BASE compile, which declares no `ClusterGrid` at all. The gate's OFF branch
    // is therefore reached here only when a boot-armed frame later sees `clusters_enabled` go
    // false at RUNTIME (`ResolvedRenderPath::froxel_light_cull` is boot-frozen, while
    // `LightHeaderGpu::new` packs the LIVE bit every frame), or when the dims/capacity terms trip.
    // The loop BODY (range
    // test, falloff, spot cone, punctual atlas shadow, BSDF accumulate) is UNCHANGED between the
    // two arms -- only the index-list SOURCE differs, so a `-D FROXEL=1` recompile cannot
    // perturb the flat-walk lighting math, and the base (non-FROXEL) compile is byte-for-byte
    // unperturbed.
#ifdef FROXEL
    float view_z = dot(cam_forward.xyz, P - cam_eye.xyz);
    ClusterParams cp = load_cluster_params(LightBuf);
    // Defense-in-depth (VB-P1b-0 C1): also require non-zero dims, mirroring
    // `cluster_cull.hlsl`'s own `cluster_count = dim_x*dim_y*dim_z` guard. Without this, a
    // header that ever carries `clusters_enabled=1` with stale/zero dims (e.g. a one-frame
    // fold/sync-gate race) would let `cluster_z_slice`/`cluster_linear_index`
    // (light_table.hlsli) underflow to a huge index -- an out-of-bounds `ClusterGrid` read
    // with `robust_buffer_access` disabled. A zero-dims header now falls back to the in-bounds
    // flat walk instead. Inert on every armed, correctly-packed frame (dims are always nonzero
    // together with the enabled bit once `sync_cluster_light_gate` has run), so ON==OFF
    // equality is unaffected. This seam compiles into BOTH `vb_shade_froxel.comp.spv` and
    // `vb_shade_tex_froxel.comp.spv` (the same source, `-D TEXTURED=1` for the latter).
    //
    // VB-P1k (the CAPACITY term): non-zero dims are not enough. `cluster_linear_index` is
    // < dim_x*dim_y*dim_z by construction, so the LIVE header's dims are the only bound this
    // read would otherwise have -- while `ClusterGrid` was SIZED at boot from
    // `ClusterConfig::cluster_count()` and is never re-allocated. `sync_cluster_light_gate`
    // republishes the LIVE `ClusterConfig` dims into the header every frame, so a post-boot
    // `ClusterConfig` edit that GROWS the grid makes this read leave the allocation --
    // silently, `robustBufferAccess` being OFF with no GPU-assisted validation. `GetDimensions`
    // reports the BOUND DESCRIPTOR's own element count (SPIR-V `OpArrayLength`), i.e. the
    // allocation itself rather than a host-side mirror of it, so the third term disarms the
    // cluster walk for exactly the frames whose live grid does not fit the buffer and falls
    // back to the in-bounds flat scan -- which is also the CORRECT lighting for such a frame,
    // whereas clamping the index would silently shade against the wrong froxel. Inert whenever
    // boot dims == live dims (every shipping configuration: `cluster_count == grid_capacity`,
    // so the term reads `n <= n`), so ON==OFF equality is again unaffected.
    uint grid_capacity, grid_stride;
    ClusterGrid.GetDimensions(grid_capacity, grid_stride);
    uint cluster_count = cp.dim_x * cp.dim_y * cp.dim_z;
    bool use_clusters = (cp.clusters_enabled != 0u) && (cluster_count != 0u)
                     && (cluster_count <= grid_capacity);
    uint ps_count;   // number of point/spot lights to walk
    uint ps_offset;  // base into LightIndexList (clusters) or the flat block
    if (use_clusters) {
        uint2 tile = cluster_xy_tile(px, py, w, h, cp);
        uint zsl = cluster_z_slice(view_z, cp);
        uint cluster = cluster_linear_index(tile.x, tile.y, zsl, cp.dim_x, cp.dim_z);
        uint2 cell = ClusterGrid[cluster];
        ps_offset = cell.x;  // offset into LightIndexList
        ps_count = cell.y;   // count of indices in this froxel's slice
    } else {
        ps_offset = H.l0a_count;                  // flat block base
        ps_count = H.light_count - H.l0a_count;    // flat block length
    }
    for (uint jj = 0u; jj < ps_count; ++jj) {
        uint j = use_clusters ? LightIndexList[ps_offset + jj] : (ps_offset + jj);
#else
    for (uint j = H.l0a_count; j < H.light_count; ++j) {
#endif
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
