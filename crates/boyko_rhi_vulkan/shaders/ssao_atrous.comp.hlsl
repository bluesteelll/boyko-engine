// Render P7 POLISH follow-up — the SSAO denoise MOVED OUT of the resolve into a dedicated
// edge-avoiding à-trous compute chain, mirroring the SHIPPED `shadow_atrous.comp.hlsl` RT
// soft-shadow denoiser (Dammertz 2010, "à trous" — "with holes").
//
// ONE dispatch per à-trous LEVEL: reads the previous level's filtered AO lane (`gAoIn` — the raw
// `sdf_ssao` gather output at level 0, the previous pass's output otherwise), writes the next
// lane (`gAoOut`), widening the filter footprint each level via the hole `step = 1 << level` (a
// push-constant). After `N` passes the host binds the FINAL lane into `deferred_pbr.hlsl`'s
// `gSsao` slot; the resolve's `ssao_mode != SSAO_MODE_OFF` combine `.Load`s it directly (a
// single `gSsao.Load`, no more inline blur) and min-combines exactly as before. With `N == 0`
// (no dispatch) the resolve reads the raw `sdf_ssao` gather output verbatim — the byte-identical
// OFF path (the SSAO-off pinned golden is untouched either way: the pass is gated on
// `scene.ssao.is_some()`, orthogonal to the à-trous level count).
//
// # The filter (Dammertz canonical 5-tap B3-spline — TRANSCENDENTAL-FREE)
//
// A 2D 5x5 B3-spline kernel `h = (1/16, 1/4, 3/8, 1/4, 1/16)` (outer product -> 25 taps),
// COMPILE-TIME-CONSTANT width (the 25 taps unroll). The ONLY runtime variation is the hole
// `step` (the push-const): tap `t` samples the pixel at `center + o * step` for `o` in the ±2
// grid, COORDINATE-CLAMPED to the image bounds (never skipped — an edge tap reuses the border
// pixel, mirroring `shadow_atrous`'s clamp). UNLIKE `shadow_atrous`'s `exp`/`pow` edge-stops,
// this filter stays TRANSCENDENTAL-FREE (integer/mul/div/clamp/min/max only) so the bit-exact
// host oracle (`golden_ssao_atrous`, `boyko_rhi_vulkan::goldens`) survives: the depth gate is
// the SAME plane-fit RESIDUAL gate + polynomial `w_depth` falloff `deferred_pbr.hlsl`'s retired
// inline blur used (Render P7 POLISH Change C), now gating a LINEAR-Z residual (see below) with
// the predicted offset `dz_pred` scaled by the pass's hole `step` (the SVGF `phiDepth =
// gradient*gStepSize` convention):
//   dz      = z_t - z_c - dz_pred                                     // plane-fit residual
//   if (|dz| > SSAO_BLUR_DEPTH_TOL) continue;                         // hard silhouette gate
//   w_depth = clamp(1 - dz*dz/SSAO_BLUR_DEPTH_SIGMA^2, 0, 1)          // polynomial falloff
//   w       = SSAO_ATROUS_H[ox+2]*SSAO_ATROUS_H[oy+2] * w_depth       // B3 kernel x depth
//   sum += w * s;  wsum += w;
// Output: `out = wsum > SSAO_ATROUS_W_EPS ? sum/wsum : s_c` (the center-fallback guard — the
// center tap ALWAYS self-passes with weight `SSAO_ATROUS_H[2]^2 == 0.140625`, so `wsum` never
// hard-zeros; the guard only protects the float division itself).
//
// # Linear view-Z reconstruction (VERBATIM COPY of `shadow_atrous.comp.hlsl::linear_view_z`)
//
// The depth gate compares LINEAR view depth, reconstructed IDENTICALLY to `shadow_atrous` /
// the resolve's `csm_view_z`: PERSPECTIVE `z = dot(rd, cam_forward.xyz) * view_t` (rd from the
// SHARED `ray_gen.hlsli` per pixel), ORTHO `z = view_t` (a no-op — for the bit-exact ORTHO test
// fixtures `linear_view_z(view_t) == view_t`, so the switch from the old raw-`view_t` gate is
// numerically free there). At a step-4 hole (±8 px) the raw ray-param `t`'s screen gradient is a
// poor first-order fit near frame edges where `rd` diverges from `cam_forward`; linear-Z avoids
// spuriously rejecting coplanar taps. The camera params arrive through the SAME 80-byte
// `cbuffer Camera` the resolve/SSAO/marcher/`shadow_atrous` use.
//
// # Resources (dedicated 3-image + 1-UBO bind-group; role-keyed by format, see boyko_app/gpu_scene)
//
//   binding 0 : RWTexture2D<float> (STORAGE) — gAoIn  (READ;  the previous lane)
//   binding 1 : RWTexture2D<float> (STORAGE) — gAoOut (WRITE; this pass's lane)
//   binding 2 : RWTexture2D<float> (STORAGE, r32f) — gViewT (READ; surface ray-param t)
//   binding 3 : cbuffer Camera (UNIFORM) — the 80-byte extent/camera block
// A 4-byte `[[vk::push_constant]]` block carries the current level's hole `step` (= 1 << level).
// `[[vk::image_format]]` pins each storage image's OpTypeImage. Deliberately DROPS
// `shadow_atrous`'s `gNormal` (the SSAO gate is depth-plane-fit only, no normal edge-stop — a
// normal edge-stop needs `pow`, banned here) and its `ResolvedShadowDenoise` UBO (the tunables
// stay baked `static const`, no owner-retunable UBO for this pass).
//
// # Three format-pin variants (the R8<->R16 ping-pong bridge; format lives ONLY in the pin, the
//   bind-group LAYOUT is IDENTICAL across all three — only the bound VIEW + the pin differ)
//
// The gather writes `gSsao` as R8_UNORM (the frozen resolve-read format) and the resolve reads
// it back as R8_UNORM; the intermediate à-trous passes use R16_UNORM rings (anti-banding —
// mirrors `shadow_atrous`'s uniform-RG16 ping-pong design, here single-channel). So the à-trous
// chain BRIDGES formats only at its two ends:
//   -D SSAO_ATROUS_READ_R8=1  : gAoIn  pinned "r8"  (level 0 only — reads the R8 gather output)
//   -D SSAO_ATROUS_WRITE_R8=1 : gAoOut pinned "r8"  (the LAST level only — writes back into gSsao)
//   (neither defined)         : gAoIn/gAoOut both pinned "r16" (every interior level)
// The eDSL-GENERATED tap span + the loop/gradient/normalize glue are BYTE-IDENTICAL across all
// three variants; only the two `[[vk::image_format]]` pins (and thereby the OpTypeImage) change.
//
// Compiled offline (hermetic — no SDK at `cargo build` time) with:
//   dxc -spirv -T cs_6_0 -E main "-fspv-target-env=vulkan1.3" \
//       ssao_atrous.comp.hlsl -Fo ssao_atrous.comp.spv                          (interior, r16/r16)
//   dxc -spirv -T cs_6_0 -E main "-fspv-target-env=vulkan1.3" -D SSAO_ATROUS_READ_R8=1 \
//       ssao_atrous.comp.hlsl -Fo ssao_atrous_read8.comp.spv                    (level 0, r8/r16)
//   dxc -spirv -T cs_6_0 -E main "-fspv-target-env=vulkan1.3" -D SSAO_ATROUS_WRITE_R8=1 \
//       ssao_atrous.comp.hlsl -Fo ssao_atrous_write8.comp.spv                   (last level, r16/r8)

#if defined(SSAO_ATROUS_READ_R8)
[[vk::image_format("r8")]] RWTexture2D<float> gAoIn : register(u0);
#else
[[vk::image_format("r16")]] RWTexture2D<float> gAoIn : register(u0);
#endif

#if defined(SSAO_ATROUS_WRITE_R8)
[[vk::image_format("r8")]] RWTexture2D<float> gAoOut : register(u1);
#else
[[vk::image_format("r16")]] RWTexture2D<float> gAoOut : register(u1);
#endif

// binding 2: the ray-param t lane the depth gate reconstructs linear-Z from (the marcher's
// store view, in GENERAL).
[[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(u2);

// binding 3: the camera/extent UNIFORM block — byte-identical field layout to the marcher /
// resolve / SSAO / `shadow_atrous` `Camera` (and the host `CompositePushConstants`).
cbuffer Camera : register(b3) {
    uint   count;        // total pixel count = img_w * img_h
    uint   img_w_raw;    // runtime extent width  (0 => IMG_W_DEFAULT)
    uint   img_h_raw;    // runtime extent height (0 => IMG_H_DEFAULT)
    uint   camera_mode;  // RAYGEN_CAM_ORTHO | RAYGEN_CAM_PERSPECTIVE
    float4 cam_eye;      // xyz = eye world pos          (PERSPECTIVE)
    float4 cam_forward;  // xyz = forward basis, w = tan(fovY/2) (PERSPECTIVE)
    float4 cam_right;    // xyz = right basis,  w = aspect (W/H)  (PERSPECTIVE)
    float4 cam_up;       // xyz = up basis                (PERSPECTIVE)
};

// The à-trous hole width for THIS level's dispatch (= 1 << level). Mirrors the host
// `SsaoAtrousPush`.
struct SsaoAtrousPush {
    uint step;
};
[[vk::push_constant]] SsaoAtrousPush pc;

// Shared camera ray-generation (the SAME header the marcher / resolve / SSAO / `shadow_atrous`
// include — ONE ray-gen, no drift). Reconstructs `rd` for the PERSPECTIVE linear-view-Z term.
#include "ray_gen.hlsli"

// The legacy 64x64 fixture extent when the UBO extent is zero (mirrors the marcher/resolve/SSAO).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// The mesh/SDF G-buffer background sentinel (mirror the marcher's gViewT `1.0e30` sentinel).
static const float VIEWT_BG = 1.0e30;

// The Dammertz 5-tap B3-spline weights (EXACT f32; identical table to `shadow_atrous`'s
// `ATROUS_H`) for offsets -2..+2. The 2D kernel is the outer product `H[ox+2] * H[oy+2]`.
static const float SSAO_ATROUS_H[5] = { 0.0625, 0.25, 0.375, 0.25, 0.0625 };

// The per-pass normalization guard: below this accumulated weight the center pixel is a
// hard-edge island (every neighbour depth-gated away) -> pass its own value through unfiltered.
static const float SSAO_ATROUS_W_EPS = 1.0e-4;

// The plane-fit RESIDUAL depth gate (view-Z units) — the hard silhouette guard (unchanged from
// the retired resolve blur, Render P7 POLISH Change C).
static const float SSAO_BLUR_DEPTH_TOL = 1.0;
// The depth-weight polynomial falloff scale (view-Z units).
static const float SSAO_BLUR_DEPTH_SIGMA = 1.0;
// The per-pixel linear-Z gradient clamp (view-Z per pixel; plane-fit slope cap).
static const float SSAO_BLUR_GRAD_CLAMP = 0.1;

// Linear view depth for pixel (px, py), reconstructed BIT-CONSISTENT with `shadow_atrous`'s /
// the resolve's `csm_view_z`: PERSP `dot(rd, cam_forward.xyz) * view_t`, ORTHO `view_t` (`rd`
// from the shared ray-gen; `cam_forward.xyz` is contractually normalized). VERBATIM COPY of
// `shadow_atrous.comp.hlsl::linear_view_z` (see this file's header doc, "Linear view-Z
// reconstruction").
float linear_view_z(uint px, uint py, uint w, uint h, float view_t) {
    if (camera_mode == RAYGEN_CAM_PERSPECTIVE) {
        float3 ro, rd;
        generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);
        return dot(rd, cam_forward.xyz) * view_t;
    }
    return view_t;
}

// Loads `gViewT` at `(px, py)` and reconstructs its linear-Z — the gradient/tap neighbour-read
// primitive (hand-written glue; not eDSL).
float linear_view_z_load(uint px, uint py, uint w, uint h) {
    return linear_view_z(px, py, w, h, gViewT.Load(int2((int)px, (int)py)));
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

    // Center taps: the AO sample + the linear-Z gate reference.
    float s_c      = gAoIn.Load(coord);
    float view_t_c = gViewT.Load(coord);
    float z_c      = linear_view_z(px, py, w, h, view_t_c);

    // The slope-aware (plane-fit) depth-gate gradient — min-magnitude ONE-SIDED linear-Z
    // differences from the 4 direct (±1, UNSCALED by `step`) neighbours (coordinate-CLAMPED
    // loads: an image-edge "neighbour" reads the center's own texel, a zero diff), clamped to
    // ±SSAO_BLUR_GRAD_CLAMP. Min-magnitude picks the in-surface side at a silhouette (the tie
    // keeps the +side).
    float z_xp = linear_view_z_load(min(px + 1, w - 1), py, w, h);
    float z_xm = linear_view_z_load(px == 0 ? 0u : px - 1, py, w, h);
    float z_yp = linear_view_z_load(px, min(py + 1, h - 1), w, h);
    float z_ym = linear_view_z_load(px, py == 0 ? 0u : py - 1, w, h);
    float grad_xp = z_xp - z_c;
    float grad_xm = z_c - z_xm;
    float grad_yp = z_yp - z_c;
    float grad_ym = z_c - z_ym;
    float dzdx = clamp((abs(grad_xp) > abs(grad_xm)) ? grad_xm : grad_xp,
                        -SSAO_BLUR_GRAD_CLAMP, SSAO_BLUR_GRAD_CLAMP);
    float dzdy = clamp((abs(grad_yp) > abs(grad_ym)) ? grad_ym : grad_yp,
                        -SSAO_BLUR_GRAD_CLAMP, SSAO_BLUR_GRAD_CLAMP);

    // The à-trous accumulate over the 5x5 B3-spline holes at stride `pc.step`.
    float ssao_sum = 0.0;
    float ssao_wsum = 0.0;
    [unroll]
    for (int oy = -2; oy <= 2; ++oy) {
        [unroll]
        for (int ox = -2; ox <= 2; ++ox) {
            // Tap coordinate at the à-trous hole `o * step`, clamped to the image bounds (an edge
            // tap reuses the border pixel — the sampler-address-clamp analogue for a UAV point read).
            int tx = clamp((int)px + ox * (int)pc.step, 0, (int)w - 1);
            int ty = clamp((int)py + oy * (int)pc.step, 0, (int)h - 1);
            int2 tcoord = int2(tx, ty);

            float s = gAoIn.Load(tcoord);
            float z_t = linear_view_z_load((uint)tx, (uint)ty, w, h);
            float h_weight = SSAO_ATROUS_H[ox + 2] * SSAO_ATROUS_H[oy + 2];
            // SVGF step-scaled predicted linear-Z offset.
            float dz_pred = dzdx * float(ox * (int)pc.step) + dzdy * float(oy * (int)pc.step);

            // === GENERATED ssao_atrous_tap BEGIN ===
            float dz = z_t - z_c - dz_pred;
            if (abs(dz) > SSAO_BLUR_DEPTH_TOL) {
                continue;
            }
            float depth_sigma2 = SSAO_BLUR_DEPTH_SIGMA * SSAO_BLUR_DEPTH_SIGMA;
            float w_depth = clamp(1.0 - dz * dz / depth_sigma2, 0.0, 1.0);
            float w = h_weight * w_depth;
            ssao_sum = ssao_sum + w * s;
            ssao_wsum = ssao_wsum + w;
            // === GENERATED ssao_atrous_tap END ===
        }
    }

    // Normalize; guard the hard-edge island (all neighbours gated away -> ssao_wsum below the
    // floor) by passing the center value through unfiltered.
    float out_ao = (ssao_wsum > SSAO_ATROUS_W_EPS) ? (ssao_sum / ssao_wsum) : s_c;
    gAoOut[uint2(px, py)] = out_ao;
}
