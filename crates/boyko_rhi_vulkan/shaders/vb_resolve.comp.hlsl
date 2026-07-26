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
// VB-SV0 (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §4.3): a SOURCE-level `#define`, never a `-D` on the
// dxc command line, so it adds ZERO compile variants. It arms `vb_geom_fetch.hlsli`'s
// `tri_p0`/`tri_p1`/`tri_p2` members (the covered triangle's three world positions — register
// copies, no ALU) plus the `vb_sv0_face_normal` leaf that turns them into the geometric
// shadow-origin normal. Only the SV0 shadow block calls that leaf, and only from inside its
// runtime gate, so a dark frame pays for neither. `vb_geo.comp.hlsl` (the fourth includer)
// deliberately does NOT define `VB_SV0` and preprocesses character-identical to its pre-SV0 form.
// Under the kill switch the define itself must vanish along with everything else it arms, which is
// why it sits inside the guard rather than beside it.
#ifndef VB_SV0_KILL
#define VB_SV0
#endif
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

// === VB-SV0 — the SDF soft-shadow + contact-AO seam (docs/VB-SV0-SDF-SHADOW-PLAN.md) ==========
//
// Binding 10 (Set 0): the SDF edit-list SSBO — the SAME `scene.edit_list` buffer the marcher and
// the deferred resolve already bind (`deferred_pbr.hlsl:161` declares it at the identical Vulkan
// binding for the identical reason). Slot 10 and NOT slot 8: slots 8/9 are `ClusterGrid` /
// `LightIndexList` under `#ifdef FROXEL`, so 8 is free only in scenes that never arm the froxel
// cull — a silent, scene-config-dependent collision no validation layer on this box would report
// (validation is off; `robustBufferAccess` is off). Slot 10 is free in BOTH `vb_layout0` and
// `vb_layout0_froxel`. A 5th descriptor set is not an alternative: the TEXTURED VB variant
// already consumes all four sets and Vulkan's guaranteed `maxBoundDescriptorSets` floor is 4.
//
// `register(t0)` SPACE 0 is free in all three tails: `vb_resolve` uses t5/u6/t12/s12/t14/s14
// only, and `vb_shade`/`vb_shade_split` declare their UNBOUNDED `gTextures[]` at t0 in SPACE 3 —
// a different space, so no collision. A collision would be a hard `dxc` error, not a silent one.
//
// No new upload and no new barrier. The mechanism is NOT the marcher's — under `legs: Mesh`, and
// on every VB configuration where the SDF marcher is never dispatched, there is no marcher upload
// and no marcher barrier to inherit. The real write site is
// `boyko_app/src/runner.rs:1401-1416`: on the FIRST frame whose `SdfEditStaging::is_dirty()` is
// true (step "5-pre", before that same frame's `render_gbuffer_frame`), the host writes the whole
// edit list ONCE through the HostVisibleCoherent MAPPED pointer of the buffer
// `GpuSceneBundles::boot` minted, then calls `mark_uploaded()`. Visibility to the dispatch comes
// from `vkQueueSubmit`'s own IMPLICIT host-write memory dependency — coherent memory needs no
// `vkFlushMappedMemoryRanges` and no `VK_ACCESS_HOST_WRITE_BIT` barrier for writes made before the
// submit — and the write runs under the fenced dispatcher token, so it cannot race an in-flight
// read either. The R11 tripwire that keeps it one-shot is `is_dirty()` being false from frame 2
// onward; if that ever stops holding, the write becomes per-frame and DOES need a barrier
// argument. `scene.edit_list` is a plain (non-`Option`) field, valid on EVERY VB boot including
// `legs: Mesh`, so the descriptor is always writable even when nothing ever reads it.
#ifndef VB_SV0_KILL
[[vk::binding(10, 0)]] StructuredBuffer<uint> Buf : register(t0);

// `sdf_field.hlsli`'s INCLUDE CONTRACT requires `Buf` in scope FIRST (above). This tail is a
// strict FIELD-CONSUMER: it CALLS `field_distance` read-only and never edits. `field_distance`
// walks `min(Buf[0], MAX_SDF_EDITS)` edits — ALREADY clamped, so SV0 introduces no new indexing
// and no new out-of-range surface.
#include "sdf_field.hlsli"

// The shadow-march tuning block — a VERBATIM mirror of `deferred_pbr.hlsl:466-474`, which itself
// mirrors the marcher's frozen A1 consts, so the VB march matches the Deferred one it ports.
// `sdf_shadow_leaves.hlsli`'s INCLUDE CONTRACT requires `MAX_IT`/`SHADOW_K`/`SHADOW_MINT`/
// `SHADOW_MINT_STEP`/`SHADOW_HIT_EPS` in scope; `GRAD_H`/`FIELD_LIPSCHITZ_L` come from
// `sdf_field.hlsli` above. `T_MAX` is the directional caster's march bound.
static const float EPS                = 0.001;
static const float T_MAX              = 10.0;
static const uint  MAX_IT             = 128u;
static const float SHADOW_K           = 8.0;
static const float SHADOW_MINT        = 16.0 * GRAD_H;
static const float SHADOW_MINT_STEP   = 16.0 * GRAD_H;
static const float SHADOW_HIT_EPS     = 2.0 * EPS;
static const float SHADOW_NDOTL_EPS   = 0.0;
static const float SHADOW_NORMAL_BIAS = 0.02; // normal-offset march-origin lift (anti grazing-acne)

#include "sdf_shadow_leaves.hlsli"
#endif // VB_SV0_KILL

// Multi-paradigm render-path plan, rungs VB-P1a (the seam) + VB-P1b (armed): the froxel
// cluster-grid pair, Set 0 bindings 8/9 -- compiled in ONLY for the `-D FROXEL` variant
// (`vb_resolve_froxel.comp.spv`); the base (non-FROXEL, this file's default) compile never
// declares them, so its Set 0 stays byte-identical at 8 bindings and `vb_resolve.comp.spv` stays
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
#ifndef VB_SV0_KILL
    // VB-SV0: the 2-bit runtime gate (light-header word 7, bits 5..6), hoisted ONCE per pixel —
    // a wave-uniform header read, never a per-light one. Bit 5 arms the shadow, bit 6 the contact
    // AO, and they arm INDEPENDENTLY, which is why these are two separate blocks and not one:
    // each half must be able to move pixels ON ITS OWN, so neither can hide behind the other.
    //
    // Rung S2 ships this DARK — the host writes mode 0 on every configuration. There are exactly
    // THREE SV0 spans in this tail's per-pixel path, and when the mode is 0 NONE of them runs:
    //
    //   1. the contact-AO block below, gated on `VB_SDF_MESH_AO_BIT`;
    //   2. the soft-shadow block in the directional loop, gated on `VB_SDF_MESH_SHADOW_BIT`;
    //   3. the geometric face-normal chain (`cross`/`dot`/`rsqrt`/orientation flip), which lives
    //      in `vb_sv0_face_normal` and is CALLED FROM INSIDE span 2 — never from the geometry
    //      fetch. It used to sit in the fetch's straight-line code, where it ran on every covered
    //      pixel with the gate at 0; that is the dark cost this rung's review removed.
    //
    // So the only thing the dark path pays is this one wave-uniform header read plus its two
    // `if`s. The binding-10 descriptor stays bound-but-unread and every frame is byte-identical to
    // its pre-SV0 pin.
    uint sv0_mode = load_vb_sdf_mesh_mode(LightBuf);
    if ((sv0_mode & VB_SDF_MESH_AO_BIT) != 0u) {
        // Contact AO: the 5-tap field-deficit AO along the SHADING normal, `min`-combined into
        // `ao_final` — the SAME routing Deferred uses for the marcher's mesh-AO lane
        // (`deferred_pbr.hlsl:785` -> `:936` -> `:965`), so `spec_ao` below inherits it through
        // the existing formula rather than through a second, forkable expression. No origin bias:
        // the taps start at `h = AO_STEP`, already off-surface. `min` on floats is exact and
        // order-independent, so this combine commutes with every other `ao_final` combine.
        ao_final = min(ao_final, sdf_ao(P, n));
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
#ifndef VB_SV0_KILL
                // VB-SV0: the SDF soft shadow on mesh, `min`-combined into the PRIMARY
                // directional's `vis` beside the CSM combine above — and deliberately OUTSIDE the
                // `csm_mode` `if`, because the two shadow sources arm independently and an
                // SDF-shadow scene need not also run cascades.
                //
                // `NoL > SHADOW_NDOTL_EPS` stands in for `sdf_soft_shadow`'s own back-face
                // early-out; the RANGED leaf has none (its `n` parameter is unread — the caller
                // owns the early-out, see the leaf's doc), so the caller must gate. At `NoL == 0`
                // the direct term is multiplied by `NoL` anyway, so skipping the march is
                // behaviourally identical and strictly cheaper.
                //
                // The march ORIGIN is lifted along the GEOMETRIC face normal (§4.2), not the
                // shading normal: computed from actual world positions it is the true plane
                // normal under any affine instance transform, and it removes the classic
                // silhouette self-shadow acne. Exactly ONE march per covered pixel regardless of
                // light count — this is the primary directional only.
                //
                // `vb_sv0_face_normal` is called HERE, inside the gate, and not in
                // `vb_geom_fetch`: the fetch exports only the three world positions (register
                // copies SROA erases), so the `cross`/`dot`/`rsqrt`/flip chain exists solely on
                // the armed path. See that function's doc — the placement is disassembly-verified
                // on the committed `.spv`, not assumed to survive `-O3`.
                if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u && NoL > SHADOW_NDOTL_EPS) {
                    float3 sv0_face_n = vb_sv0_face_normal(geo);
                    vis = min(vis, sdf_soft_shadow_ranged(P + sv0_face_n * SHADOW_NORMAL_BIAS, n, l, T_MAX));
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
    // gated on the header's `clusters_enabled` bit (`use_clusters`) -- an armed frame maps this
    // pixel to its froxel and walks ONLY the survivors `cluster_cull.hlsl` wrote into
    // `ClusterGrid`/`LightIndexList` (the SAME lookup `forward_opaque.fs.hlsl`'s own FROXEL arm
    // performs); an unarmed frame (or the base, non-FROXEL compile) falls back to the IDENTICAL
    // flat `[l0a_count, light_count)` scan, TOKEN-FOR-TOKEN the SAME clone of
    // `forward_opaque.fs.hlsl`'s own non-FROXEL arm this file always ran. The loop BODY (range
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
    // equality is unaffected.
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
#ifdef VB_SV0_ULP_PROBE
    // VB-SV0 G2 SENSITIVITY CONTROL (plan §3.3, S2 gate (f)). NEVER a shipping variant: no
    // manifest row, no embed, no pipeline, and no host code defines this. It exists so that
    // "every OFF-path golden is byte-identical" can be shown to be a gate that CAN go red rather
    // than one that is merely satisfied.
    //
    // It perturbs the final `lit` by exactly ONE ULP AFTER the OETF — downstream of the exposure
    // scale, the tonemap and the gamma encode. That placement is the whole point: a 1-ULP
    // perturbation of an UPSTREAM shading term was measured byte-invisible to this golden (§11.1)
    // precisely because those three attenuators sit between it and the store. None of them is
    // between this and the store.
    //
    // Honest limits, stated so a BLIND result is read correctly rather than explained away.
    //
    // BLIND SET — the probe runs BEFORE the `clamp(lit, 0, 1)` below, so it is not just the
    // channels EXACTLY at 1.0/0.0 that cannot register: it is EVERY channel with `lit >= 1.0` or
    // `lit <= 0.0`, i.e. every saturated pixel, since the clamp maps a whole half-line onto one
    // byte. Any blown-out highlight and any fully-shadowed region is invisible to this probe.
    //
    // SENSITIVITY — the mechanism that decides whether an UNSATURATED channel registers is the
    // ratio of the perturbation to the 8-bit quantisation step. One ULP near 1.0 is ~1.2e-7; the
    // store's quantisation step is 1/255 = 3.9e-3. So the probe flips a byte only where a channel
    // already sits within ~1.2e-7 / 3.9e-3 ≈ 1/32000 of a rounding boundary. Over ~1M pixels × 3
    // channels that is on the order of 100 expected byte crossings — tiny, but far from zero, and
    // it is a COUNT that a byte-compare either sees or does not.
    //
    // If the run still comes back BLIND, that is §7 clause 1's finding — the golden cannot
    // resolve a 1-ULP change even at the store — and NOT a reason to widen the probe.
    lit = asfloat(asuint(lit) + uint3(1u, 1u, 1u));
#endif
    gLit[uint2(px, py)] = float4(clamp(lit, 0.0, 1.0), 1.0);
}
