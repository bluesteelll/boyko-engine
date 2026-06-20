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
//
// 7 STORAGE/uniform bindings — within the 12-binding cap (5 free; the cap was raised
// 8 → 12 in the Lighting L0 plan). All four images are consumed in GENERAL (the marcher's
// STORAGE views, kept in GENERAL after a memory-only COMPUTE→COMPUTE barrier) and `gLit`
// is a storage store. `[[vk::image_format("rgba8")]]` pins each `OpTypeImage` to `Rgba8`
// (shaderStorageImageWriteWithoutFormat is OFF).
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

// Shared camera ray-gen (the SAME header the marcher includes — ONE ray-gen, no drift).
#include "ray_gen.hlsli"
// Shared light-table std430 decode (ONE source of truth, included by the resolve + cull).
#include "light_table.hlsli"

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

        float3 lit_direct = float3(0.0, 0.0, 0.0);
        float3 ambient = float3(0.0, 0.0, 0.0);
        for (uint i = 0u; i < H.l0a_count; ++i) {
            LightElem L = load_light(LightBuf, i);
            if (L.kind == LIGHT_KIND_DIRECTIONAL) {
                float3 l = normalize(L.dir);
                float3 hvec = normalize(v + l);
                float NoL = max(dot(n, l), 0.0);
                float NoH = saturate(dot(n, hvec));
                float LoH = saturate(dot(l, hvec));
                float  D = D_GGX(NoH, a);
                float  V = V_SmithGGXCorrelated(NoV, NoL, a); // folds 1/(4 NoL NoV)
                float3 F = F_Schlick(LoH, f0);
                float3 spec = (D * V) * F;
                float3 diff = diffuse_color * (1.0 / PI);
                lit_direct += (diff + spec) * (NoL * shadow) * L.color;
            } else if (L.kind == LIGHT_KIND_SKY) {
                // Hemisphere ambient: lerp(ground, sky, hemi) diffuse + EnvBRDFApprox spec.
                float3 sky_color = L.color;       // upper hemisphere
                float3 ground_color = L.pos;      // lower hemisphere (packed in pos lane)
                float2 dfg = env_brdf_approx(roughness, NoV);
                float3 hemi_color = lerp(ground_color, sky_color, hemi);
                float3 spec_ambient = (f0 * dfg.x + dfg.y) * sky_color;
                float3 diff_ambient = diffuse_color * hemi_color;
                ambient += (spec_ambient + diff_ambient) * ao;
            }
            // Point/spot (kinds 1/2) are the L0b block — not in the L0a front block.
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
