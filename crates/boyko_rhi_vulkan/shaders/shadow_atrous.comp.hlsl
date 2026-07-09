// Render Shadow Rung 3a — the edge-avoiding à-trous shadow-visibility denoise (Dammertz 2010).
//
// ONE dispatch per à-trous LEVEL: reads the ping visibility image (`shadow_vis` / `shadow_vis2`),
// writes the pong, widening the filter footprint each level via the hole `step = 1 << level` (a
// push-constant). The input is the RAW per-pixel mesh visibility the `deferred_pbr.hlsl` VIS stage
// wrote (R = mesh_vis, G = validity: 1 = a real mesh-shadow sample, 0 = the neutral fill on a pixel
// that never reached the mesh arm). The VIS trace is a single-frame Vogel-disk cone sample, so its
// penumbra reads GRAINY; this spatial filter smooths the grain WHILE preserving the shadow shape by
// stopping the blur across geometry edges (normal + linear-view-Z discontinuities) — no TAA / no
// history (the engine's no-temporal convention), so the smoothness is purely spatial.
//
// After `levels` iterations the host binds the FINAL pong into `deferred_pbr.hlsl`'s `gShadowVis`
// slot; the RESOLVE_DENOISED stage `.Load`s it and min-combines exactly as RESOLVE_INLINE did with
// its inline trace. With `levels == 0` (no dispatch) the RESOLVE_DENOISED read sees the raw VIS
// output verbatim, so the render is bit-identical to RESOLVE_INLINE (the C3 algebraic anchor).
//
// # The filter (Dammertz "Edge-Avoiding À-Trous Wavelet Transform")
//
// A 2D 5x5 B3-spline kernel `h = (1/16, 1/4, 3/8, 1/4, 1/16)` (outer product → 25 taps),
// COMPILE-TIME-CONSTANT width (the 25 taps unroll). The ONLY runtime variation is the hole `step`
// (the push-const): tap `t` samples the pixel at `center + o * step` for `o` in the ±2 grid. Per
// tap the weight is the kernel weight `h_x*h_y` modulated by two edge-stop functions + the tap's
// validity, so the blur never bleeds shadow across a silhouette:
//   w_n   = pow(max(0, dot(n_t, n_c)), sigma_n)                         // normal edge-stop
//   w_z   = exp(-|z_t - z_c| / (sigma_z * length(o*step) + eps))        // linear-view-Z edge-stop
//   w     = h_x*h_y * w_n * w_z * valid_t                               // valid_t gates dead taps
//   sum_v += w * vis_t;  sum_w += w
// Output: `vis = sum_w > eps ? sum_v/sum_w : vis_c`, `validity = sum_w > eps ? 1 : valid_c` (`sum_w`
// already folds each tap's validity, so it alone drives the normalization + the validity gate). NOT
// separable (the non-linear edge-stop would streak along the separated axis). `sigma_z / sigma_n`
// come from the `ResolvedShadowDenoise` UBO (owner-retunable, live).
//
// # Linear view-Z reconstruction (BIT-CONSISTENT with `deferred_pbr.hlsl::csm_view_z`)
//
// The Z edge-stop compares LINEAR view depth, reconstructed IDENTICALLY to the resolve's
// `csm_view_z`: PERSPECTIVE `z = dot(rd, cam_forward.xyz) * view_t` (rd from the SHARED `ray_gen.hlsli`
// per pixel), ORTHO `z = view_t`. A background / non-lit tap (`view_t >= VIEWT_BG`, validity 0) is
// gated out by `valid_t == 0` (its weight is zeroed), so its garbage z/normal never perturbs the
// filter. The camera params (mode + basis) arrive through the SAME 80-byte `cbuffer Camera` the
// resolve/SSAO/marcher use; `ray_gen.hlsli` reconstructs `rd` binding-agnostically.
//
// # Resources (dedicated 6-binding denoise bind-group)
//
//   binding 0 : RWTexture2D<float2> (STORAGE, rg16)     — gVisIn  (READ; the ping visibility)
//   binding 1 : RWTexture2D<float2> (STORAGE, rg16)     — gVisOut (WRITE; the pong visibility)
//   binding 2 : RWTexture2D<float4> (STORAGE, rgba8)    — gNormal (READ; oct + material id)
//   binding 3 : RWTexture2D<float>  (STORAGE, r32f)     — gViewT  (READ; surface ray param t)
//   binding 4 : cbuffer ResolvedShadowDenoise (UNIFORM) — sigma_z / sigma_n (16 B)
//   binding 5 : cbuffer Camera (UNIFORM)                — the 80-byte extent/camera block
// A 4-byte `[[vk::push_constant]]` block carries the current level's hole `step` (= 1 << level).
// `[[vk::image_format]]` pins each storage image's OpTypeImage (shaderStorageImageWriteWithoutFormat
// is OFF). BOTH ping-pong rings are R16G16_UNORM (uniform-RG16 design), so gVisIn AND gVisOut are
// pinned `rg16` — the single pin matches the bound view on every level, whichever ring is the READ
// (even ⇒ shadow_vis, odd ⇒ shadow_vis2) and whichever is the WRITE (the other).
//
// Compiled offline (hermetic — no SDK at `cargo build` time) with:
//   dxc -spirv -T cs_6_0 -E main "-fspv-target-env=vulkan1.3" \
//       shadow_atrous.comp.hlsl -Fo shadow_atrous.comp.spv

// bindings 0..1: the ping/pong visibility images. gVisIn is read (the previous level's output — the
// raw VIS image at level 0); gVisOut is written. BOTH ping-pong rings (`shadow_vis` + `shadow_vis2`)
// are R16G16_UNORM (uniform-RG16 design — 16-bit avoids cumulative 8-bit rounding across levels), so
// BOTH views are pinned `[[vk::image_format("rg16")]]`; the single pin matches the bound view on
// every parity (even ⇒ read shadow_vis / write shadow_vis2, odd ⇒ the reverse).
[[vk::image_format("rg16")]] RWTexture2D<float2> gVisIn  : register(u0);
[[vk::image_format("rg16")]] RWTexture2D<float2> gVisOut : register(u1);
// bindings 2..3: the G-buffer lanes the edge-stops read (the marcher's store views, in GENERAL).
[[vk::image_format("rgba8")]] RWTexture2D<float4> gNormal : register(u2);
[[vk::image_format("r32f")]]  RWTexture2D<float>  gViewT  : register(u3);

// binding 4: the edge-stop scalars UBO — byte-mirrors `boyko_render::ResolvedShadowDenoise` (16 B,
// std140: two f32 + 8 B pad = one vec4 slot). `sigma_z` scales the view-Z edge-stop falloff (larger
// = the blur crosses depth steps more freely); `sigma_n` is the normal edge-stop power (larger = the
// blur stops harder at a normal discontinuity). Owner-retunable each frame (live).
cbuffer ResolvedShadowDenoise : register(b4) {
    float sigma_z;   // view-Z edge-stop scale
    float sigma_n;   // normal edge-stop power
    float _pad0;     // std140 pad to the 16-byte stride
    float _pad1;
};

// binding 5: the camera/extent UNIFORM block — byte-identical field layout to the marcher /
// resolve / SSAO `Camera` (and the host `CompositePushConstants`). Used for the extent (the pixel
// grid) + the per-pixel view direction (the shared ray-gen), for the LINEAR view-Z reconstruction.
cbuffer Camera : register(b5) {
    uint   count;        // total pixel count = img_w * img_h
    uint   img_w_raw;    // runtime extent width  (0 => IMG_W_DEFAULT)
    uint   img_h_raw;    // runtime extent height (0 => IMG_H_DEFAULT)
    uint   camera_mode;  // RAYGEN_CAM_ORTHO | RAYGEN_CAM_PERSPECTIVE
    float4 cam_eye;      // xyz = eye world pos          (PERSPECTIVE)
    float4 cam_forward;  // xyz = forward basis, w = tan(fovY/2) (PERSPECTIVE)
    float4 cam_right;    // xyz = right basis,  w = aspect (W/H)  (PERSPECTIVE)
    float4 cam_up;       // xyz = up basis                (PERSPECTIVE)
};

// The à-trous hole width for THIS level's dispatch (= 1 << level). The ONLY runtime variation in the
// otherwise compile-time-constant 25-tap kernel. A 4-byte push (the RHI `push_constant_bytes`
// minimum). Mirrors the host `ShadowAtrousPush`.
struct ShadowAtrousPush {
    uint step;
};
[[vk::push_constant]] ShadowAtrousPush pc;

// Shared camera ray-generation (the SAME header the marcher / resolve / SSAO include — ONE ray-gen,
// no drift). Reconstructs `rd` for the PERSPECTIVE linear-view-Z term, binding-agnostically.
#include "ray_gen.hlsli"

// The legacy 64x64 fixture extent when the UBO extent is zero (mirrors the marcher/resolve/SSAO).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// The mesh/SDF G-buffer background sentinel (mirror the marcher's gViewT `1.0e30` sentinel). A tap
// at/above this has no surface; its validity is already 0, so it is gated out of the filter.
static const float VIEWT_BG = 1.0e30;
// The Z edge-stop denominator floor (Dammertz's `+ eps`): keeps the center tap (`|o*step| == 0`)
// from a 0/0 and bounds the near-tap falloff.
static const float ATROUS_Z_EPS = 1.0e-2;
// The normalization guard: below this accumulated weight the center pixel is a hard-edge island (all
// neighbours edge-stopped away) → pass its own value through unfiltered.
static const float ATROUS_W_EPS = 1.0e-4;

// The 5-tap B3-spline weights `h = (1/16, 1/4, 3/8, 1/4, 1/16)` for offsets -2..+2. The 2D kernel is
// the outer product `h[ox+2] * h[oy+2]`.
static const float ATROUS_H[5] = { 0.0625, 0.25, 0.375, 0.25, 0.0625 };

// --- Octahedral decode (BYTE-IDENTICAL to the resolve's `oct_decode` in deferred_pbr.hlsl; the
// inverse of the marcher's oct_encode). gNormal.rg carries the octahedral normal; the normal
// edge-stop measures `dot(n_tap, n_center)`.
float3 oct_decode(float2 e) {
    e = e * 2.0 - 1.0;                          // [0,1] -> [-1,1]
    float3 n = float3(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    float t = saturate(-n.z);
    n.x += n.x >= 0.0 ? -t : t;
    n.y += n.y >= 0.0 ? -t : t;
    return normalize(n);
}

// Linear view depth for pixel (px, py), reconstructed BIT-CONSISTENT with the resolve's
// `csm_view_z`: PERSP `dot(rd, cam_forward.xyz) * view_t`, ORTHO `view_t` (`rd` from the shared
// ray-gen; `cam_forward.xyz` is contractually normalized).
float linear_view_z(uint px, uint py, uint w, uint h, float view_t) {
    if (camera_mode == RAYGEN_CAM_PERSPECTIVE) {
        float3 ro, rd;
        generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);
        return dot(rd, cam_forward.xyz) * view_t;
    }
    return view_t;
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

    // Center taps: the visibility (R) + validity (G) + the edge-stop references (normal, linear z).
    float2 center_vis = gVisIn.Load(coord);
    float  vis_c      = center_vis.r;
    float  valid_c    = center_vis.g;
    float3 n_c        = oct_decode(gNormal.Load(coord).rg);
    float  view_t_c   = gViewT.Load(coord);
    float  z_c        = linear_view_z(px, py, w, h, view_t_c);

    // The à-trous accumulate over the 5x5 B3-spline holes at stride `pc.step`. `sum_w` already folds
    // each tap's validity (a dead tap contributes weight 0), so the normalization + the output
    // validity gate below both key off `sum_w` alone — no separate valid accumulator is needed.
    float sum_v = 0.0;
    float sum_w = 0.0;
    [unroll]
    for (int oy = -2; oy <= 2; ++oy) {
        [unroll]
        for (int ox = -2; ox <= 2; ++ox) {
            // Tap coordinate at the à-trous hole `o * step`, clamped to the image bounds (an edge tap
            // reuses the border pixel — the sampler-address-clamp analogue for a UAV point read).
            int tx = clamp((int)px + ox * (int)pc.step, 0, (int)w - 1);
            int ty = clamp((int)py + oy * (int)pc.step, 0, (int)h - 1);
            int2 tcoord = int2(tx, ty);

            float2 tap_vis = gVisIn.Load(tcoord);
            float  vis_t   = tap_vis.r;
            float  valid_t = tap_vis.g;
            float3 n_t     = oct_decode(gNormal.Load(tcoord).rg);
            float  view_t  = gViewT.Load(tcoord);
            float  z_t     = linear_view_z((uint)tx, (uint)ty, w, h, view_t);

            // Normal edge-stop: `pow(max(0, dot(n_t, n_c)), sigma_n)` — a divergent normal (a
            // silhouette) drives the weight toward 0, stopping the blur across the geometry edge.
            float w_n = pow(max(0.0, dot(n_t, n_c)), sigma_n);
            // Linear-view-Z edge-stop: `exp(-|z_t - z_c| / (sigma_z * length(o*step) + eps))` — a
            // depth step (a far/near surface) drives the weight toward 0. The `length(o*step)`
            // scaling widens the tolerance for far holes (Dammertz's spatial normalization).
            float2 o_step = float2((float)(ox * (int)pc.step), (float)(oy * (int)pc.step));
            float w_z = exp(-abs(z_t - z_c) / (sigma_z * length(o_step) + ATROUS_Z_EPS));
            // The B3-spline kernel weight (outer product) × the two edge-stops × the tap validity.
            // `valid_t == 0` (a neutral / background tap) zeroes the whole term, so a dead tap never
            // contributes its garbage visibility/normal/z.
            float h_xy = ATROUS_H[ox + 2] * ATROUS_H[oy + 2];
            float wt = h_xy * w_n * w_z * valid_t;

            sum_v += wt * vis_t;
            sum_w += wt;
        }
    }

    // Normalize the filtered visibility; guard the hard-edge island (all neighbours edge-stopped away
    // → `sum_w` below the floor) by passing the center value + validity through unfiltered. When at
    // least one valid tap contributed, the output validity is 1 (a real filtered sample).
    bool has_weight = sum_w > ATROUS_W_EPS;
    float out_vis   = has_weight ? (sum_v / sum_w) : vis_c;
    float out_valid = has_weight ? 1.0 : valid_c;
    gVisOut[uint2(px, py)] = float2(out_vis, out_valid);
}
