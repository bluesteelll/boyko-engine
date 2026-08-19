// sdf_mesh_shadow.comp — VB-SV0 DP1 (`docs/VB-SV0-SDF-SHADOW-PLAN.md` Rev 10): the DEDICATED
// screen-space SDF-on-mesh shadow + contact-AO prepass — `docs/RENDER-PARITY-PLAN.md` §3.2's
// Option B, the architecture the critic agreed to before the inline detour.
//
// # Why a dedicated pass, in two numbers
//
// The inline form (rung S2, reverted at `13f1c9a3`) compiled this march into all ten VB
// lit-producer tails. Carrying it DARK cost ~+75% of the fused `vb_resolve` dispatch (24576 →
// 41984 ns, exact repeat) on every VB frame with the feature off; and ARMED, the S5 A/B measured
// the march at 2.342× its own cost in the Deferred marcher — a ratio that contains neither march
// ALU nor occupancy (both cancel by construction in that A/B), i.e. the VB shading tail is a
// 2.34× worse HOST for this work. This pass is the marcher-shaped host: one march per covered
// pixel, in a module whose working set is the FIELD, not the shading stack.
//
// # What it computes, and where the tails pick it up
//
// Per covered pixel (`vb_id != SENTINEL`): the §4.1 leaf pair over the frozen field —
// `R` = `sdf_soft_shadow_ranged` from the GEOMETRIC face normal's lifted origin (§4.2), for the
// PRIMARY directional (the first `LIGHT_KIND_DIRECTIONAL` in `l0a` order — the same selection the
// tails' own loop makes with `primary_dir_seen`); `G` = the 5-tap `sdf_ao` along the SHADING
// normal. The tails `min`-combine `R` into the primary directional's `vis` beside their CSM
// combine and `min`-combine `G` into `ao_final` (DP2) — the identical routing Deferred uses for
// its own decoupled `gMaterial.RG` term, which is the parity this pass restores.
//
// # Sentinel / miss handling
//
// Misses write NOTHING — the tails' read sites live strictly inside their own covered-pixel flow
// (they early-return on the same sentinel before sampling), so a miss texel is never read and the
// "misses write nothing" ownership contract of `vb_resolve`/`sdf_forward_march` carries over.
//
// # The mode bits are read HERE too, per-bit
//
// DP3 records this pass only when the resolved mode is non-zero (structural absence when off),
// but the two halves arm INDEPENDENTLY (§3.1: bit 5 shadow, bit 6 AO), so each block still gates
// on its own bit: a shadow-only arm must not pay the AO taps, and vice versa.
//
// # Compile (offline + hermetic; committed `.spv` is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//     -fspv-target-env=vulkan1.3 sdf_mesh_shadow.comp.hlsl -Fo sdf_mesh_shadow.comp.spv
//
// # Set / binding vocabulary (its own layout — NOT the tails' `vb_layout0`)
//
//   Set 0:
//     b0/u0: StructuredBuffer<VbInstanceRow>  gVbInstances  (`vb_geom_fetch.hlsli`)
//     b2   : cbuffer Camera                                 the shared 80-byte extent/camera block
//     b3   : StructuredBuffer<uint>           LightBuf      Lighting L0 light table
//     b5   : Texture2D<uint2>                 gVbId         the `vb_id` R32G32_UINT (`.Load`)
//     b6   : RWTexture2D<float2> (rg8)        gSdfTerm      the R8G8 term this pass OWNS
//     b10  : StructuredBuffer<uint>           Buf           the SDF edit list (§2.2's slot-10
//            reconciliation — free in every VB layout, and the same slot `deferred_pbr.hlsl`
//            chose for the same reason)
//   Set 2: the geometry table (`vb_geom_fetch.hlsli`'s own `gMeshVerts[]`/`gMeshIndices[]`/
//          `gMeshMeta` — identical to every other fetch consumer).
//
//   Binding numbers are deliberately the TAILS' numbers for the resources the tails also bind
//   (0/2/3/5), so a reader diffing this file against `vb_resolve.comp.hlsl` sees the pass's own
//   surface (6, 10) rather than a renumbering.

// `VB_SV0` is a SOURCE-level `#define` (never a `-D`): it unlocks `vb_geom_fetch.hlsli`'s
// `tri_p0/1/2` exports and `vb_sv0_face_normal`. The ten lit-producer tails do NOT define it and
// preprocess character-identical to their pre-SV0 form — byte-identity by construction.
#define VB_SV0

#include "vb_pack.hlsli"
#include "vb_geom_fetch.hlsli"

// binding 2: the extent/camera UNIFORM block — the SAME 80-byte shape every other consumer
// (`vb_resolve.comp.hlsl`'s Camera @2, `sdf_forward_march.comp.hlsl`'s Camera @3) declares.
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

// binding 5: the `vb_id` raster output (R32G32_UINT) — SAMPLED, unfiltered `.Load` fetch.
Texture2D<uint2> gVbId : register(t5);

// binding 6: the R8G8_UNORM term target this pass OWNS — `R` shadow visibility, `G` contact AO,
// both in [0,1] with 1.0 = "no effect", mirroring Deferred's `gMaterial.RG` decoupled term.
[[vk::image_format("rg8")]] RWTexture2D<float2> gSdfTerm : register(u6);

#include "light_table.hlsli"

// binding 10: the SDF edit list. `sdf_field.hlsli`'s INCLUDE CONTRACT requires `Buf` in scope
// FIRST. This pass is a strict FIELD-CONSUMER: it CALLS `field_distance` read-only and never
// edits; `field_distance` walks `min(Buf[0], MAX_SDF_EDITS)` edits — already clamped.
[[vk::binding(10, 0)]] StructuredBuffer<uint> Buf : register(t0);
#include "sdf_field.hlsli"

// The shadow-march tuning block — a VERBATIM mirror of `deferred_pbr.hlsl`'s, which itself
// mirrors the marcher's frozen A1 consts, so this march matches the Deferred one it ports.
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

// The 64-byte push constant — the geometry-fetch reprojection matrix (same shape as the tails').
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

    uint2 packed = gVbId.Load(int3((int)px, (int)py, 0));
    VbId id = vb_id_unpack(packed);
    if (id.instance_id == VB_ID_SENTINEL) {
        return;
    }

    float2 pixel_xy = float2((float)px, (float)py) + 0.5; // pixel-CENTER, matching SV_Position
    float2 extent = float2((float)w, (float)h);
    VbGeomFetchResult geo = vb_geom_fetch(id.instance_id, id.raw_prim_id, pixel_xy, pc.view_proj, extent);

    float3 n = normalize(geo.world_normal);
    float3 P = geo.world_pos;

    // Hoisted ONCE per pixel — a wave-uniform header read (§3.1, word 7 bits 5..6).
    uint sv0_mode = load_vb_sdf_mesh_mode(LightBuf);

    float vis = 1.0;
    if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u) {
        // The PRIMARY directional: the first `LIGHT_KIND_DIRECTIONAL` in `l0a` order — the same
        // light the tails' `primary_dir_seen` latch selects, so the term this pass writes is the
        // term the inline would have computed, for the same caster. Exactly ONE march per covered
        // pixel regardless of light count.
        LightHeader H = load_light_header(LightBuf);
        for (uint i = 0u; i < H.l0a_count; ++i) {
            LightElem L = load_light(LightBuf, i);
            if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
                float3 l = normalize(L.dir);
                float NoL = dot(n, l);
                // `NoL > SHADOW_NDOTL_EPS` stands in for the leaf's back-face early-out; the
                // RANGED leaf has none (its `n` parameter is unread — the caller owns the
                // early-out). At `NoL <= 0` the tails multiply the direct term by `NoL` anyway,
                // so leaving `vis` at 1.0 is behaviourally identical and strictly cheaper.
                if (NoL > SHADOW_NDOTL_EPS) {
                    // March ORIGIN lifted along the GEOMETRIC face normal (§4.2), not the shading
                    // normal: from actual world positions it is the true plane normal under any
                    // affine instance transform, and it removes silhouette self-shadow acne.
                    float3 sv0_face_n = vb_sv0_face_normal(geo);
                    vis = sdf_soft_shadow_ranged(P + sv0_face_n * SHADOW_NORMAL_BIAS, n, l, T_MAX);
                }
                break;
            }
        }
    }

    float ao = 1.0;
    if ((sv0_mode & VB_SDF_MESH_AO_BIT) != 0u) {
        // Contact AO: the 5-tap field-deficit AO along the SHADING normal. No origin bias: the
        // taps start at `h = AO_STEP`, already off-surface.
        ao = sdf_ao(P, n);
    }

    gSdfTerm[uint2(px, py)] = float2(vis, ao);
}
