// R9 VB geo/shade split plan (docs/R9-VB-SPLIT-PLAN.md, rung R9d, Section 3/5): the split's
// HARDWARE MESH-SHADOW VISIBILITY producer (cfg hwrt). The thin-aux mirror of
// `deferred_pbr.hlsl`'s `SHADOW_STAGE_VIS` arm (that file's own doc, ~1059-1095): traces the
// SAME Vogel-disk soft-shadow cone against the per-frame TLAS for the primary directional
// light and writes the RAW (un-denoised) visibility to `gShadowVis`, which the framegraph's
// `tlas -> shadow_vis -> shadow_atrous x N -> shadow_temporal` chain (docs/R9-VB-SPLIT-PLAN.md
// Section 3) filters before `vb_shade_split.comp.hlsl`'s `#if HWRT` arm consumes the FINAL
// filtered lane (a single `.Load` + min-combine, mirroring `deferred_pbr.hlsl`'s own
// `SHADOW_STAGE_RESOLVE_DENOISED` arm). This pass never shades a pixel and never touches
// `gLit` -- a pure visibility producer, paired 1:1 with `vb_geo.comp.hlsl` (the OTHER
// thin-aux producer) but its OWN NEW descriptor-set layout: unlike `vb_geo`, this pass does
// not reuse `vb_layout0` -- it needs neither the VB instance/material/geometry tables nor
// `gVbId`, only the thin-aux G-buffer (`gThinNormal`/`gViewT`, the deferred fat-G-buffer's
// thin equivalent), the light table, the camera, the TLAS, and the shadow tuning UBO.
//
// # Sentinel / background handling (the VB_THIN precedent, `sdf_ssao.comp.hlsl`'s own header)
//
// The thin-normal path has NO `gMaterial` (no analytic-shadow/AO/mask G-buffer -- the
// no-matcache rule, docs/R9-VB-SPLIT-PLAN.md Section 4): the mesh/background test is
// `view_t >= 1.0e30` ALONE (the marcher/`vb_viewt` background sentinel), read off the
// already-bound `gViewT` -- mirrors `sdf_ssao.comp.hlsl`'s `-D VB_THIN=1` mask arm ("the mask
// test becomes `view_t < SSAO_VIEWT_BG` alone"). EVERY in-bounds pixel is seeded
// `float2(1.0, 0.0)` FIRST (R = full visibility, G = validity 0 -- "no real sample was
// written"), exactly as `deferred_pbr.hlsl`'s `SHADOW_STAGE_VIS` arm does (that file's
// `main()`, ~782-788); a background pixel keeps this seed and returns before the trace. The
// Vogel core OVERWRITES it with `float2(mesh_vis, 1.0)` only when a primary directional casts
// a shadow onto the pixel (CSM armed AND `NoL > 0`) -- the IDENTICAL predicate the deferred
// VIS arm gates on.
//
// # The Vogel-disk cone trace (VERBATIM copy of `deferred_pbr.hlsl`'s `SHADOW_STAGE_VIS` arm)
//
// `sh_up`/`sh_tx`/`sh_ty` (the orthonormal basis around the light axis `l`), `sh_rot =
// ign(px,py)*6.2831853 + float(SHADOW_FRAME_SEED & 0xFFu)*2.399963229728653` (the per-pixel
// IGN spiral + the rung-3b per-frame golden-angle temporal-decorrelation term), the `[loop]`
// over `SHADOW_RAY_COUNT` `rayQuery` inline traces (`ACCEPT_FIRST_HIT_AND_END_SEARCH |
// FORCE_OPAQUE | SKIP_PROCEDURAL_PRIMITIVES` -- an occlusion query, first hit suffices), and
// `mesh_vis = 1.0 - occ / SHADOW_RAY_COUNT` are copied CHARACTER-IDENTICAL from
// `deferred_pbr.hlsl` (that file's own doc: "copied VERBATIM so `mesh_vis` is bit-identical to
// the inline path") -- see that file's header doc for the full soft-shadow rationale.
// `ign`/`oct_decode` are duplicated pure helpers here (the `forward_opaque.fs.hlsl`
// "Duplicated, not shared" precedent this codebase already establishes for every compute-pass
// shading tail, e.g. `vb_resolve.comp.hlsl`'s `env_brdf_approx`/`sun_kernel*`); `oct_decode` is
// BYTE-IDENTICAL to `deferred_pbr.hlsl`'s/`sdf_ssao.comp.hlsl`'s own copy, the inverse of
// `vb_geo.comp.hlsl`'s `oct_encode`.
//
// # Primary-directional selection (a trimmed mirror of `deferred_pbr.hlsl`'s `primary_dir_seen`)
//
// This pass traces ONLY the primary directional (the FIRST `LIGHT_KIND_DIRECTIONAL` entry in
// the light table's l0a front block) -- the SAME light the deferred resolve's `primary_dir_
// seen` gate singles out for the mesh-shadow term. Unlike the resolve (which keeps looping to
// shade every light), this producer's job ends once the primary is found: the loop `break`s
// immediately after processing it, whether or not the CSM/NoL gate fired.
//
// # Bindings (Set 0 ONLY, a NEW dense 7-binding layout -- no shared `vb_layout0` reuse)
//
//   binding 0 (u0): RWTexture2D<float4> (rgba8) gThinNormal -- READ. `vb_geo.comp.hlsl`'s thin
//                   aux normal (RG = oct-encoded world vertex normal; B = roughness, A = 1.0,
//                   both UNREAD here -- only RG is decoded).
//   binding 1 (u1): RWTexture2D<float>  (r32f)  gViewT      -- READ. The VB thin-aux surface
//                   ray param `t` (the `vb_viewt`/marcher background-sentinel convention,
//                   `1.0e30` on a miss); doubles as the mesh/background mask (no `gMaterial`).
//   binding 2 (t2): StructuredBuffer<uint>        LightBuf   -- READ. The Lighting-L0 light
//                   table (`light_table.hlsli`'s word-indexed decode); this pass reads ONLY the
//                   header + the l0a front block to find the primary directional.
//   binding 3 (b3): cbuffer Camera                            -- READ. The SAME 80-byte
//                   extent/camera shape every other consumer (`vb_geo`/`vb_resolve`/
//                   `sdf_ssao.comp.hlsl`) declares -- the extent bounds guard + the shared
//                   ray-gen.
//   binding 4 (t4): RaytracingAccelerationStructure tlas       -- READ. The per-frame TLAS
//                   (R2a-3 `PersistentTlas.accel`), the SAME acceleration structure
//                   `deferred_pbr.hlsl`'s `#if HWRT` arm traces.
//   binding 5 (b5): cbuffer RayShadowUbo                       -- READ. BYTE-IDENTICAL field
//                   shape to `deferred_pbr.hlsl`'s own (`boyko_render::ResolvedRayShadow` +
//                   the runner-injected `SHADOW_FRAME_SEED`): cone_radius@0, tmax@4, tmin@8,
//                   bias@12, frame_seed@16 -- one UBO, the SAME shared upload path.
//   binding 6 (u6): RWTexture2D<float2> (rg16)  gShadowVis  -- WRITE. RG: R = raw mesh
//                   visibility, G = validity (1 = a real sample, 0 = the neutral background
//                   seed) -- the SAME format + semantics `deferred_pbr.hlsl`'s `gShadowVis`
//                   (binding 21) carries, consumed by the SAME à-trous denoiser + temporal
//                   reproject chain.
//
// `SHADOW_RAY_COUNT` is `[[vk::constant_id(0)]]` (spec-const, default 16 -- BYTE-IDENTICAL to
// `deferred_pbr.hlsl`'s own), baked at pipeline build. NO push constant -- every input this
// pass needs is a binding.
//
// Compiled offline (hermetic build -- no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_5 -E main \
//       -fspv-target-env=vulkan1.3 vb_shadow_vis.comp.hlsl -Fo vb_shadow_vis.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_shadow_vis.comp.spv

// binding 0/1: the VB thin-aux G-buffer (`vb_geo.comp.hlsl`'s own output; the no-matcache mask
// is `gViewT` ALONE, this file's header doc).
[[vk::image_format("rgba8")]] RWTexture2D<float4> gThinNormal : register(u0);
[[vk::image_format("r32f")]]  RWTexture2D<float>  gViewT      : register(u1);

// binding 2: the Lighting-L0 light table (word-indexed `[LightHeaderGpu || GpuLight[]]`).
StructuredBuffer<uint> LightBuf : register(t2);

// binding 3: the extent/camera UNIFORM block -- byte-identical shape to every other consumer's.
cbuffer Camera : register(b3) {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};

// binding 4: the per-frame TLAS (R2a-3 `PersistentTlas.accel`), traced by the Vogel-disk core
// below -- the SAME acceleration structure `deferred_pbr.hlsl`'s `#if HWRT` arm binds.
[[vk::binding(4)]] RaytracingAccelerationStructure tlas;

// binding 5: the tunable soft-shadow params UBO -- BYTE-IDENTICAL field shape to
// `deferred_pbr.hlsl`'s own `RayShadowUbo` (that file's doc, ~360-372).
cbuffer RayShadowUbo : register(b5) {
    float SHADOW_CONE_RADIUS; // tan(half-angle) of the sun disk, ~2 deg
    float SHADOW_RAY_TMAX;
    float SHADOW_RAY_TMIN;
    float SHADOW_RAY_BIAS;
    uint  SHADOW_FRAME_SEED;  // rung 3b: per-frame counter, offset 16 (see above)
};

// R2a-4b soft-shadow: `SHADOW_RAY_COUNT` rays are jittered on a Vogel disk within the sun's
// angular cone (`SHADOW_CONE_RADIUS`) around the light axis; the AVERAGE miss fraction is the
// soft penumbra (BYTE-IDENTICAL default to `deferred_pbr.hlsl`'s own spec-const id 0).
[[vk::constant_id(0)]] const uint SHADOW_RAY_COUNT = 16;

// binding 6: the shadow-visibility output -- RG: R = raw mesh_vis, G = validity. Consumed by
// the à-trous denoiser + the temporal reproject, exactly like `deferred_pbr.hlsl`'s own.
[[vk::image_format("rg16")]] RWTexture2D<float2> gShadowVis : register(u6);

// Shared camera ray-generation (the SAME header the marcher/resolve/`vb_resolve` include).
#include "ray_gen.hlsli"
// Shared light-table std430 decode (ONE source of truth, `light_table.hlsli`).
#include "light_table.hlsli"

// The mesh/background sentinel (mirrors `sdf_ssao.comp.hlsl`'s `SSAO_VIEWT_BG` / the
// marcher/`vb_viewt` convention). The thin-normal path has no `gMaterial` mask -- `view_t`
// alone decides.
static const float SHADOW_VIS_VIEWT_BG = 1.0e30;

// --- Octahedral decode (BYTE-IDENTICAL to `deferred_pbr.hlsl`'s/`sdf_ssao.comp.hlsl`'s own
// copy -- the inverse of `vb_geo.comp.hlsl`'s `oct_encode`). `gThinNormal.rg` carries the
// oct-encoded world vertex normal.
float3 oct_decode(float2 e) {
    e = e * 2.0 - 1.0;                          // [0,1] -> [-1,1]
    float3 n = float3(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    float t = saturate(-n.z);
    n.x += n.x >= 0.0 ? -t : t;
    n.y += n.y >= 0.0 ? -t : t;
    return normalize(n);
}

// Interleaved-Gradient Noise (BYTE-IDENTICAL to `deferred_pbr.hlsl`'s own `ign`) -- the
// per-pixel hash the Vogel-disk spiral rotates by.
float ign(uint px, uint py) {
    float3 magic = float3(0.06711056, 0.00583715, 52.9829189);
    return frac(magic.z * frac(dot(float2((float)px, (float)py), magic.xy)));
}

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

    // Rung 3a VIS precedent (`deferred_pbr.hlsl`'s `SHADOW_STAGE_VIS` arm): seed the NEUTRAL
    // visibility (full vis, validity 0) for EVERY in-bounds pixel FIRST. A pixel that never
    // reaches the trace (background, or a primary directional the CSM/NoL gate skips) keeps
    // this -- validity 0 tells the à-trous filter the texel carries no real sample.
    gShadowVis[uint2(px, py)] = float2(1.0, 0.0);

    int2 coord = int2((int)px, (int)py);
    float view_t = gViewT.Load(coord);
    if (view_t >= SHADOW_VIS_VIEWT_BG) {
        // Background: the thin-normal path has no `gMaterial` mask (the no-matcache rule) --
        // `view_t` alone is the mesh/background test (the VB_THIN precedent).
        return;
    }

    float3 n = oct_decode(gThinNormal.Load(coord).rg);

    float3 ro, rd;
    generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);
    float3 P = ro + rd * view_t;

    // The primary directional (the FIRST directional in the l0a front block) is the ONLY light
    // this pass traces (this file's header doc, "Primary-directional selection").
    LightHeader H = load_light_header(LightBuf);
    uint csm_mode = load_csm_mode(LightBuf);
    for (uint i = 0u; i < H.l0a_count; ++i) {
        LightElem L = load_light(LightBuf, i);
        if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
            float3 l = normalize(L.dir);
            float NoL = max(dot(n, l), 0.0);
            // CSM Increment 1b gate (`deferred_pbr.hlsl`'s own predicate, ~1004): a back-faced
            // receiver is already `NoL == 0`, so the trace is skipped.
            if (csm_mode != CSM_MODE_OFF && NoL > 0.0) {
                // === VERBATIM Vogel-disk cone trace (`deferred_pbr.hlsl`'s
                // `SHADOW_STAGE_VIS` arm, that file's own doc ~1059-1095) ===
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
            }
            break; // this producer's job ends at the primary directional
        }
    }
}
