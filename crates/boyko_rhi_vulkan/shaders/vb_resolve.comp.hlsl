// Multi-paradigm render-path plan, rung R8: the VisibilityBuffer FUSED resolve compute pass
// (`mesh_geo_shade_split == false` — SSAO/DDGI/shadow-denoise/TAA all structurally capped off
// this rung, `cap_vb_v1_consumers`). One full-screen dispatch, one thread per pixel: unpacks
// `vb_id` -> re-fetches the covered triangle's geometry (`vb_geom_fetch.hlsli`, Decision 0/9) ->
// shades it against the full light table (ALL-LIGHTS, no froxel — mirrors plain `Forward`'s own
// non-FROXEL base compile) -> writes `lit` directly (Decision 5: pure-VB, no material
// G-buffer/cache — every pixel re-fetches AND re-shades from scratch, the Burns/Hable
// recompute-for-bandwidth trade).
//
// # Sentinel / miss handling
//
// `instance_id == VB_ID_SENTINEL` marks a pixel `vb_raster` never covered (the sky background,
// or — once the SDF leg composites under VB, R10 — an SDF-owned pixel): this pass WRITES
// NOTHING for such a pixel, so whatever `vb_sky` already painted this frame stands (the SAME
// "misses write nothing" contract `sdf_forward_march.comp.hlsl`'s ownership gate documents).
//
// # Shading (cloned from `forward_opaque.fs.hlsl`, token-for-token where noted)
//
// After a valid fetch, `Surface` is built from the re-fetched world normal + the instance's
// per-instance material id (`instance_materials[instance_id].id`, the SAME ring
// `forward_opaque.vs.hlsl` indexes — Decision 0's "material comes from the EXISTING
// per-instance material ring, not the geometry fetch" contract), then the ALL-LIGHTS direct
// loop + `eval_pbr_ambient_hemi`/`eval_pbr_sun_disc` + the tonemap tail are a TOKEN-FOR-TOKEN
// clone of `forward_opaque.fs.hlsl`'s own loop (CSM/atlas visibility + attenuation shapes
// unchanged; NO baked SDF shadow term this rung — mesh-only, `vis` starts at the Deferred-mesh-
// pixel constant `1.0`, exactly like Forward v1's own scope cut). `env_brdf_approx`/
// `sun_kernel*`/`SUN_ENV_WEIGHT` are duplicated here too (see `forward_opaque.fs.hlsl`'s
// "Duplicated (not shared) pure helpers" doc for the rationale — a compute pass cannot share a
// raster FS's local helpers via anything but textual duplication in this codebase's `.hlsl`
// authoring model).
//
// # Material (v1 scope cut — NON-TEXTURED ONLY)
//
// `base_color`/`metallic`/`roughness`/`emissive` come from the `Materials` SSBO (`MaterialGpu`,
// byte-identical shape to every other consumer), keyed by the per-instance material ring's
// `id`. `#ifdef TEXTURED` is a seam for a later rung (mirrors `gbuffer_mrt.fs.hlsl`'s/
// `forward_opaque.fs.hlsl`'s own TEXTURED seam) — `vb_geom_fetch`'s reconstructed `uv` is ready
// (perspective-correct) but UNREAD by this v1 shading tail; no bindless texture table is bound.
//
// # Bindings
//
//   Set 0 (VB core + images — a NEW, VB-only layout, `vb_layout0`; NOT `forward_layout0`):
//     b0/u0: StructuredBuffer<VbInstanceRow>      gVbInstances    (`vb_geom_fetch.hlsli`)
//     t1   : StructuredBuffer<PerInstanceMaterial> instance_materials
//     b2   : cbuffer Camera                                       80-byte extent/camera block
//                                                                  (SAME shape as every other
//                                                                  consumer's Camera UBO —
//                                                                  binding 2 chosen so the
//                                                                  EXISTING `forward_sky`
//                                                                  pipeline's compiled SPIR-V
//                                                                  is reusable verbatim against
//                                                                  a NEW pipeline object built
//                                                                  against THIS layout, see
//                                                                  `declare_vb_graph`'s doc)
//     t3   : StructuredBuffer<uint>                LightBuf        Lighting L0 light table
//                                                                  (binding 3, same reuse
//                                                                  rationale as Camera @2)
//     t4   : StructuredBuffer<MaterialGpu>          Materials      PBR material table
//     t5   : Texture2D<uint2>                       gVbId          the `vb_id` R32G32_UINT
//                                                                  raster output (SAMPLED,
//                                                                  `.Load` unfiltered fetch)
//     u6   : RWTexture2D<float4> (rgba8)             gLit           the shared `lit` target
//   Set 1 (shadow) is a VERBATIM copy of `forward_opaque.fs.hlsl`'s own Set-1 block (CSM
//   cascades + punctual atlas) — REUSES `ForwardTargets`'s bind-group-layout OBJECT verbatim
//   (`GBufferScene::forward_layout1`, the SAME "no new layout" reuse `sdf_forward_march.comp.hlsl`'s
//   Set 1 already establishes), so the SAME physical descriptor set binds to every
//   Forward-family/VB shading pass.
//   Set 2 (geometry, Decision 0/P2-c) — `vb_geom_fetch.hlsli`'s own `gMeshVerts[]`/
//   `gMeshIndices[]`/`gMeshMeta` (see that file's doc for the Set-numbering deviation from the
//   plan's literal "Set 3" text).
//
// # Push constant (64 bytes — the geometry-fetch reprojection matrix)
//
//   offset 0: float4x4 view_proj   the SAME reverse-Z proj*view `vb_raster.vs.hlsl` used
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_resolve.comp.hlsl -Fo vb_resolve.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_resolve.comp.spv

// binding 0 (`gVbInstances`, the `VbInstanceRow` SSBO) is declared inside `vb_geom_fetch.hlsli`
// itself (that file's own INCLUDE CONTRACT — self-contained, needs nothing pre-declared).
#include "vb_pack.hlsli"
#include "vb_geom_fetch.hlsli"

// binding 1: the per-instance material payload — `boyko_render::mesh_draw::PerInstanceMaterial`
// (32 B), the SAME struct `forward_opaque.vs.hlsl` reads. This pass reads ONLY `.id` (the
// v1 non-textured material path sources albedo from `Materials[mat_id].base_color`).
struct PerInstanceMaterial {
    float4 base_color;
    uint   id;
    uint3  _pad;
};
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;

// binding 2: the extent/camera UNIFORM block — the SAME 80-byte shape every other consumer
// (`forward_opaque.fs.hlsl`'s Camera @2, `sdf_forward_march.comp.hlsl`'s Camera @3) declares.
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

// binding 4: the material table — byte-identical `MaterialGpu` shape to every other consumer.
struct MaterialGpu {
    float4 base_color;
    float4 mrr;
    float4 emissive;
};
[[vk::binding(4, 0)]] StructuredBuffer<MaterialGpu> Materials;

// binding 5: the `vb_id` raster output (R32G32_UINT) — SAMPLED, unfiltered `.Load` fetch (no
// sampler consumed; mirrors `sdf_forward_march.comp.hlsl`'s `gForwardDepth.Load` idiom).
Texture2D<uint2> gVbId : register(t5);

// binding 6: the shared `lit` STORAGE image — the SAME physical image `vb_sky`/(a future mesh
// raster leg) write via a COLOR attachment; C5's COLOR_ATTACHMENT -> GENERAL chain.
[[vk::image_format("rgba8")]] RWTexture2D<float4> gLit : register(u6);

// --- Shading (cloned from `forward_opaque.fs.hlsl` — see this file's header doc) -----------

// `pbr_lighting.hlsli`'s INCLUDE CONTRACT precondition: `PI` + `LIGHT_UP` in scope first.
static const float PI = 3.14159265358979323846;
static const float3 LIGHT_UP = float3(0.0, 1.0, 0.0);

#include "pbr_lighting.hlsli"
#include "light_table.hlsli"

// --- Set 1 (shadow): a VERBATIM copy of `forward_opaque.fs.hlsl`'s own Set-1 block, so the SAME
// physical descriptor set (`ForwardTargets::set1`) binds to both the Forward-family raster/
// compute passes and this one.

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
    uint2 _gCsmPad;
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

// --- Duplicated pure helpers (VERBATIM copy of `forward_opaque.fs.hlsl`'s own span — see that
// file's "Duplicated (not shared) pure helpers" doc for the rationale) ----------------------

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

// The 64-byte push constant — the geometry-fetch reprojection matrix (see this file's header).
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
    // visible after `vb_raster`'s color write — no unsynchronized access.
    uint2 packed = gVbId.Load(int3((int)px, (int)py, 0));
    VbId id = vb_id_unpack(packed);
    if (id.instance_id == VB_ID_SENTINEL) {
        // Misses write NOTHING — the sky color `vb_sky` already painted this frame stands (the
        // SAME ownership-gate contract `sdf_forward_march.comp.hlsl` documents).
        return;
    }

    float2 pixel_xy = float2((float)px, (float)py) + 0.5; // pixel-CENTER, matching SV_Position
    float2 extent = float2((float)w, (float)h);
    VbGeomFetchResult geo = vb_geom_fetch(id.instance_id, id.raw_prim_id, pixel_xy, pc.view_proj, extent);

    float3 n = normalize(geo.world_normal);
    float3 P = geo.world_pos;

    PerInstanceMaterial pm = instance_materials[id.instance_id];
    MaterialGpu m = Materials[pm.id];

    float3 v = normalize(cam_eye.xyz - P);
    float NoV = max(dot(n, v), 1e-4);

    float3 base = m.base_color.rgb;
    float metallic = m.mrr.x;
    float roughness = clamp(m.mrr.y, 0.045, 1.0); // fp32 floor, mirrors the deferred resolve
    float reflectance = m.mrr.z;
    float3 emissive = m.emissive.rgb;
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
    // constant `1.0`.
    float ao_final = 1.0;
    float spec_ao = saturate(pow(NoV + ao_final, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao_final);

    LightHeader H = load_light_header(LightBuf);
    uint csm_mode = load_csm_mode(LightBuf);
    uint punctual_shadow_mode = load_punctual_shadow_mode(LightBuf);

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
            // constant `1.0`, then CSM min-combines in.
            float vis = 1.0;
            if (!primary_dir_seen) {
                primary_dir_seen = true;
                if (csm_mode != CSM_MODE_OFF && NoL > 0.0) {
                    float view_z = dot(cam_forward.xyz, P - cam_eye.xyz);
                    vis = min(vis, csm_visibility(P, n, view_z, NoL));
                }
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
