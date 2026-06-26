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
// # BRDF (the Filament/Karis real-time convergence — single scatter)
//
//   D = GGX/Trowbridge-Reitz; V = height-correlated Smith visibility (folds 1/(4 NoL NoV));
//   F = Schlick; diffuse = Lambert (albedo/PI). Metallic-roughness:
//     f0 = lerp(0.16*reflectance^2, base, metallic);  diffuse = base*(1-metallic).
//   Direct light: one analytic directional light, modulated by the A1 shadow.
//   Ambient/IBL: analytic EnvBRDFApprox (Karis mobile) for specular + a hemisphere
//   diffuse ambient, modulated by the A2 AO. No IBL texture, no LUT (MVP-2).
//   The host oracle (`golden_deferred_resolve` in compute.rs) models this identically.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 deferred_pbr.hlsl \
//       -Fo deferred_pbr.comp.spv

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

// Shared camera ray-gen (the SAME header the marcher includes — ONE ray-gen, no drift).
#include "ray_gen.hlsli"
// Shared light-table std430 decode (ONE source of truth, included by the resolve + cull).
#include "light_table.hlsli"
// P6 R1: the FROZEN shared SDF field gateway (`field_distance`) — for the per-light analytic
// `sdf_soft_shadow_ranged` march. Included AFTER `Buf` (the include contract). A strict
// field-CONSUMER; the field math + `sdf_field.hlsli` stay BYTE-FROZEN.
#include "sdf_field.hlsli"

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

// === P6 R1 — the `t_max`-RANGED soft-shadow leaf (multi-light SDF shadows) ===============
// GENERATED by `boyko_shaderdsl::emit::emit_hlsl_sdf_soft_shadow_ranged()`; a SEPARATELY-
// named clone of the marcher's frozen `sdf_soft_shadow` whose escape break spells the RUNTIME
// `t_max` instead of the hardcoded `T_MAX` (B3 — option a). The `sdf_soft_shadow_ranged_
// matches_edsl_emit` sync pin (in `boyko_rhi_vulkan/tests/sdf_field_edsl_sync.rs`) pins this
// to the generator; a hand-edit fails CI. `t_max` = the light DISTANCE for a punctual caster
// or `T_MAX` for an extra directional. The `dot(n, L)` early-out is the resolve's per-light
// `NoL <= 0` skip (hand-written in the loop), so this body is the loop+tail only.
// === GENERATED sdf_soft_shadow_ranged BEGIN ===
float sdf_soft_shadow_ranged(float3 p, float3 n, float3 L, float t_max) {
    float res = 1.0;
    float t = SHADOW_MINT;
    [loop]
    for (uint i = 0u; i < MAX_IT; ++i) {
        float d = field_distance(p + L * t);
        res = min(res, SHADOW_K * d / t);
        if (d < SHADOW_HIT_EPS) {
            return 0.0;
        }
        t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);
        if (t > t_max) {
            break;
        }
    }
    return clamp(res, 0.0, 1.0);
}
// === GENERATED sdf_soft_shadow_ranged END ===

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

// --- Cook-Torrance / GGX terms (Filament real-time forms) -----------------------------

// GGX/Trowbridge-Reitz normal distribution. `a` is the remapped roughness (perceptual^2).
float D_GGX(float NoH, float a) {
    float a2 = a * a;
    float d = (NoH * a2 - NoH) * NoH + 1.0;     // = (NoH^2)(a2-1)+1, the stable rearrange
    return a2 / (PI * d * d);
}

// Height-correlated Smith visibility (folds the 1/(4 NoL NoV) of the specular denominator).
float V_SmithGGXCorrelated(float NoV, float NoL, float a) {
    float a2 = a * a;
    float lambdaV = NoL * sqrt((NoV - a2 * NoV) * NoV + a2);
    float lambdaL = NoV * sqrt((NoL - a2 * NoL) * NoL + a2);
    return 0.5 / max(lambdaV + lambdaL, 1e-5);
}

// Schlick Fresnel.
float3 F_Schlick(float u, float3 f0) {
    float f = pow(1.0 - u, 5.0);
    return f0 + (1.0 - f0) * f;
}

// Zero-/non-finite-guarded normalize — the FAITHFUL mirror of the host oracle's
// `boyko_sdf_math::v_normalize` (compute.rs reuses it for every golden lighting
// normalize). HLSL's intrinsic `normalize(0)` is `0/0 == NaN`, whereas the host
// returns `float3(0,0,0)`; that divergence is the L1 black-pixel bug. At a surface
// whose normal faces AWAY from a still-in-range point/spot light the half-vector
// `v + l` can be ~zero (the light direction `l` is ~opposite the view dir `v`):
// the host's `v_normalize(v+l)` yields `[0,0,0]` -> NoH = LoH = 0 -> a FINITE spec
// term that the `NoL == 0` factor then zeroes, while the GPU's `normalize(v+l)`
// yields NaN -> NaN spec -> `NaN * 0 == NaN` -> `pack_unorm(NaN) == 0` -> a pure
// BLACK pixel. Using this guard for every per-light `normalize` restores bit-parity
// with the host (the guard is byte-identical to `normalize` on all non-degenerate
// inputs, so the L0a/L0b/L1-off paths that already match are unchanged).
float3 safe_normalize(float3 a) {
    float len = sqrt(dot(a, a));
    // FLT_MIN floor + isfinite guard, matching v_normalize's
    // `len <= f32::MIN_POSITIVE || !len.is_finite()` degenerate branch.
    if (len <= 1.17549435e-38 || !isfinite(len)) {
        return float3(0.0, 0.0, 0.0);
    }
    return a / len;
}

// Karis "mobile" analytic environment BRDF approximation (no DFG LUT). Returns the
// (scale, bias) the split-sum specular IBL needs: `spec_env = f0*scale + bias`.
float2 env_brdf_approx(float roughness, float NoV) {
    const float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
    const float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
    float4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * NoV)) * r.x + r.y;
    return float2(-1.04, 1.04) * a004 + r.zw;
}

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

        // The hemisphere factor the sky lerp interpolates against (world up).
        float hemi = dot(n, LIGHT_UP) * 0.5 + 0.5;

        // L0a: loop the no-`P` front block of the table (directionals + sky). The W1
        // op-order is PINNED to the host oracle (`golden_deferred_resolve_table`):
        //   direct  += (diff + spec) * (NoL * shadow) * L.color   (accumulator from 0)
        //   ambient += (spec_ambient + diff_ambient) * ao          (accumulator from 0)
        //   lit      = (direct + ambient + emissive) * exposure     (* exposure LAST)
        // No reassociation — a degenerate 1-directional + 1-sky table at exposure 1.0 is
        // bit-identical to the old LIGHT_DIR/LIGHT_COLOR/SKY_* path.
        LightHeader H = load_light_header(LightBuf);

        // P6 R1: the resolve shadow_mode (header word 7; 0 on every pre-P6 scene → the
        // BYTE-IDENTICAL 0%-gate) + the surface world position `P` (the gViewT lane, read
        // here under `mask == 1` — the SAME reconstruction the L0b block uses below, hoisted
        // up so the directional march can use it too). The `marched` counter bounds the
        // per-pixel march to `MAX_SDF_SHADOW_CASTERS_PER_PIXEL` dominant casters (Decision 2).
        uint shadow_mode = load_shadow_mode(LightBuf);
        bool multi_light = shadow_mode != SHADOW_MODE_LEGACY;
        float view_t = gViewT.Load(coord);
        float3 P = ro + rd * view_t;
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
            float ao_class = (view_t >= 1.0e30) ? 1.0 : ao;
            ao_final = min(ao_class, gSsao.Load(coord).r);
        }

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
                } else if (multi_light && light_casts_sdf_shadow(L)
                           && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
                           && NoL > SHADOW_NDOTL_EPS) {
                    // Normal-offset start bias: lift the march origin off the surface so
                    // grazing rays clear it (anti terminator-acne). Mirrors the host `pb`.
                    vis = sdf_soft_shadow_ranged(P + n * SHADOW_NORMAL_BIAS, n, l, T_MAX);
                    marched += 1u;
                }
                float3 hvec = normalize(v + l);
                float NoH = saturate(dot(n, hvec));
                float LoH = saturate(dot(l, hvec));
                float  D = D_GGX(NoH, a);
                float  V = V_SmithGGXCorrelated(NoV, NoL, a); // folds 1/(4 NoL NoV)
                float3 F = F_Schlick(LoH, f0);
                float3 spec = (D * V) * F;
                float3 diff = diffuse_color * (1.0 / PI);
                lit_direct += (diff + spec) * (NoL * vis) * L.color;
            } else if (light_kind(L) == LIGHT_KIND_SKY) {
                // Hemisphere ambient: lerp(ground, sky, hemi) diffuse + EnvBRDFApprox spec.
                float3 sky_color = L.color;       // upper hemisphere
                float3 ground_color = L.pos;      // lower hemisphere (packed in pos lane)
                float2 dfg = env_brdf_approx(roughness, NoV);
                float3 hemi_color = lerp(ground_color, sky_color, hemi);
                float3 spec_ambient = (f0 * dfg.x + dfg.y) * sky_color;
                float3 diff_ambient = diffuse_color * hemi_color;
                ambient += (spec_ambient + diff_ambient) * ao_final;
            }
            // Point/spot (kinds 1/2) are the L0b block — not in the L0a front block.
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
        ClusterParams cp = load_cluster_params(LightBuf);
        bool use_clusters = cp.clusters_enabled != 0u;
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
                float3 spot_dir = safe_normalize(L.dir);  // world spot axis (to-light dir)
                float cosA = dot(-l, spot_dir);
                float denom = max(cones.x - cones.y, 1e-4);
                float tt = saturate((cosA - cones.y) / denom);
                atten *= tt * tt;
            }
            // The SAME Cook-Torrance direct term as the directional path, scaled by the
            // distance/cone attenuation and the light's canonical (baked-I) color. The
            // half-vector uses `safe_normalize` (host `v_normalize` parity): at a back-facing
            // surface `v + l` can be ~zero, and the intrinsic `normalize(0) == NaN` would
            // poison `spec` and (since `NaN * (NoL == 0) == NaN`) blacken the pixel.
            float3 hvec = safe_normalize(v + l);
            float NoL = max(dot(n, l), 0.0);
            float NoH = saturate(dot(n, hvec));
            float LoH = saturate(dot(l, hvec));
            float  D = D_GGX(NoH, a);
            float  V = V_SmithGGXCorrelated(NoV, NoL, a);
            float3 F = F_Schlick(LoH, f0);
            float3 spec = (D * V) * F;
            float3 diff = diffuse_color * (1.0 / PI);
            // P6 R1: `vis` DEFAULTS to `shadow` (the marcher's gMaterial.r channel) — the
            // EXACT legacy L0b/L1 point/spot modulation (`(NoL * shadow) * atten`), so a
            // `shadow_mode==0` scene is BYTE-IDENTICAL to today (the 0%-gate; the L0b/L1
            // goldens are preserved). In multi-light mode a FLAGGED caster instead gets a
            // RANGE-BOUNDED analytic march — `t_max` is the light DISTANCE (`sqrt(d2)`, the
            // common short/cheap nearby case) so the shadow ray stops AT the light (an
            // occluder past the light cannot shadow). Gated by the per-light flag + the
            // dominant-N cap + the NoL > 0 skip (a back-faced light marches nothing).
            float vis = shadow;
            if (multi_light && light_casts_sdf_shadow(L)
                && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL
                && NoL > SHADOW_NDOTL_EPS) {
                float t_max = sqrt(d2);
                // Normal-offset start bias (anti grazing-acne). Mirrors the host `pb`.
                vis = sdf_soft_shadow_ranged(P + n * SHADOW_NORMAL_BIAS, n, l, t_max);
                marched += 1u;
            }
            lit_direct += (diff + spec) * (NoL * vis) * atten * L.color;
        }

        // O3: exposure is the FINAL multiply on the accumulated LINEAR radiance.
        lit = (lit_direct + ambient + m.emissive.rgb) * H.exposure;
    } else {
        // mesh / background / empty (mask == 0): PASS THE BASE THROUGH byte-identically
        // (the 0%-gate). No PBR, no material fetch, no normal/id decode.
        lit = base;
    }

    gLit[uint2(px, py)] = float4(clamp(lit, 0.0, 1.0), 1.0);
}
