// Anti-aliasing Stage 4 — TAA (Temporal Anti-Aliasing) temporal resolve.
//
// ONE dispatch, at the resolve→present seam (the SAME seam FXAA/SMAA/SSAA occupy): reprojects
// the accumulated color history through a camera-only motion-vector reconstruction, variance-
// clips it against the current frame's 3×3 neighborhood (Salvi-style, clipped TOWARD the AABB
// center — Karis/Lottes' anti-ghosting technique), and blends with a confidence-adaptive,
// luma-weighted feedback factor. Writes BOTH the history ring (for the next frame) and `aa_out`
// (the present-blit's input). Modeled on `shadow_temporal.comp.hlsl`'s proven algorithm
// (reproject → neighborhood clamp → velocity/confidence-adaptive feedback → disocclusion
// reset), generalized scalar→RGB; `shadow_temporal.comp.hlsl` itself is UNTOUCHED (reference
// only — a separate `.spv` isolates TAA).
//
// # v1 scope (C1/C2): camera-only motion, differential (jitter-free) reprojection
//
// The motion vector is reconstructed ENTIRELY from `gViewT` (the marcher-aligned depth proxy)
// + the shader's OWN camera basis (`cam_forward`/`cam_right`/`cam_up`, b6 — which C1's
// `TaaConfig::jitter_scope == RasterAndBasis` MAY shear per-pixel) + the `MotionCam`
// `cur_view_proj`/`prev_view_proj` pair (`boyko_render::motion_cam`'s `MotionCamState::advance`,
// fed `marcher_view_proj_rows` — ALWAYS built with zero jitter, independent of the b6 shear) —
// NO per-pixel motion-vector producer is read (that per-object mesh-MV path is `hwrt`-only and
// gated on the shadow-denoise temporal mode, an unrelated feature). C2 reprojects the DIFFERENCE
// between `cur_view_proj` and `prev_view_proj` rather than `prev_view_proj` alone (see the
// reprojection block below for the formula), which is exact for a moving camera over STATIC
// geometry (the canonical TAA case) and for a fully static scene (`MV ≡ 0`: a static camera has
// `prev_view_proj == cur_view_proj` bit-for-bit, so the differential is exactly zero regardless
// of whether the ray that produced `P` was sheared). A moving MESH reprojects camera-only in v1
// (ghosts like a moving SDF body already does with the mesh shadow) — a strictly smaller,
// pre-existing-class gap, not a new regression.
//
// # The history ring `taa_hist` (R16G16B16A16_SFLOAT, GENERAL, per-FIF, cross-frame seeded)
//
//   RGB = the accumulated (blended) color
//   A   = confidence (an accumulated-frame counter, reset to 1.0 on disocclusion, capped at
//         1/MIN_BLEND so `1.0 / confidence` never overshoots below `MIN_BLEND`)
//
// Frame `fi` READS `taa_hist[1-fi]` (`gHistIn`, the C1-fix framegraph-untracked read-sibling —
// see `graph_bridge.rs`'s `taa_hist_read` declaration) and WRITES `taa_hist[fi]` (`gHistOut`).
// Kept in GENERAL throughout (`Load`-based reconstruction below, no hardware sampler on the
// history ring) — the SAME GENERAL-only discipline `shadow_temporal_hist` uses, deliberately
// reused here to avoid a NEW per-frame layout-transition pair on the cross-frame ring (the W4
// framegraph is the single biggest OFF-path hazard this campaign touches; minimizing its novel
// surface was the guiding call). `gHistIn` is therefore sampled via a 16-tap SEPARABLE bicubic
// Catmull-Rom reconstruction (4×4 `Load`s, precomputed 1D Catmull-Rom weights) rather than the
// literature's 5-tap hardware-bilinear-reduced variant (which specifically REQUIRES a
// `SHADER_READ_ONLY_OPTIMAL` combined-image-sampler read — a deliberate, documented deviation;
// both reconstruct a Catmull-Rom-filtered sample, this one just spends 16 fetches instead of 5).
//
// # Correctness under motion (the #1 sensitivity, mirroring the temporal shadow denoiser)
//
//   1. Camera MV reprojects a moving camera over static geometry EXACTLY (v1 scope, C1/C2).
//   2. Variance clip (toward the AABB center) — any residual reprojection error (a moving mesh,
//      which reprojects camera-only in v1) is clamped toward the current-frame neighborhood, so
//      a wrong reprojection is pulled toward a locally-valid value, never a double-image ghost.
//   3. Disocclusion / off-screen / host-forced reset → `blend_factor == 1.0` (full replace, the
//      single-frame fallback) — never a stale or wrong-surface accumulation.
// The ONE uncovered case is a moving MESH (reprojects camera-only) — bounded by 2+3, an HONESTLY
// FLAGGED, owner-gated v1 limitation (not a silent bug), mirroring the shadow-denoiser's own
// moving-SDF-body gap.
//
// # Byte-identity
//
// This shader is a NEW `.spv` — `AaMode::Off` (the default) records no AA pass at all (the
// present-blit samples `lit` directly), so this dispatch never runs on the OFF path. It is not
// yet bound to a live pipeline this rung (see `compute.rs`'s `taa_resolve_spirv` doc for the W5
// continuation scope) — landed compiled + embedded so the binding-layout contract below is
// pinned before the boot pipeline/descriptor-set wiring lands.
//
// Compiled offline (hermetic — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main "-fspv-target-env=vulkan1.3" \
//       taa_resolve.comp.hlsl -Fo taa_resolve.comp.spv

// binding 0 (READ): the current frame's shaded LDR color `lit` (R8G8B8A8_UNORM), bound as a
// COMBINED_IMAGE_SAMPLER — the SAME resolve→AA-input seam FXAA's `fxaa_set` binds `lit` through
// (that transition to SHADER_READ_ONLY_OPTIMAL already exists structurally for every other AA
// mode; TAA reuses it rather than introducing a new one). LINEAR sampler declared for shape-
// parity with the FXAA/SMAA precedent; every tap below uses `.Load` (exact texel, no filtering)
// since the reads are discrete per-pixel/neighborhood fetches, not a fractional-UV sample.
[[vk::binding(0, 0)]] Texture2D    gLit    : register(t0);
[[vk::binding(0, 0)]] SamplerState gLitSmp : register(s0);
// binding 1 (READ): the surface ray parameter `t` (`gViewT`, r32f) — the marcher-aligned depth
// proxy the MV reconstruction ray marches by (`P = ray_origin + ray_dir * t`).
[[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(u1);
// bindings 2..3: the `taa_hist` cross-frame history ring — READ `hist[1-fi]` (`gHistIn`, the
// framegraph's `taa_hist_read` C1-fix read-sibling) + WRITE `hist[fi]` (`gHistOut`). Both
// R16G16B16A16_SFLOAT — RGB = accumulated color, A = confidence (see the module doc).
[[vk::image_format("rgba16f")]] RWTexture2D<float4> gHistIn  : register(u2);
[[vk::image_format("rgba16f")]] RWTexture2D<float4> gHistOut : register(u3);
// binding 4 (WRITE): the final AA output `aa_out` (R8G8B8A8_UNORM) — the present-blit's input.
[[vk::image_format("rgba8")]] RWTexture2D<float4> gAaOut : register(u4);

// binding 5: the TAA tunables UBO — byte-mirrors `boyko_render::taa_state`'s host-written
// constants (16 B, std140: four f32 = one vec4 slot), owner-retunable each frame (live).
// `default_blend` is the feedback weight given to the CURRENT frame on the first accumulated
// sample after a reset (a low-confidence blend, mostly replace); `min_blend` is the STEADY-STATE
// floor (a converged/static view trusts history almost entirely); `variance_gamma` scales the
// clip AABB half-width (× σ); `_pad` keeps the 16-byte std140 stride explicit (unread).
cbuffer ResolvedTaa : register(b5) {
    float default_blend;   // feedback weight at confidence == 1 (just after a reset)
    float min_blend;       // steady-state feedback weight floor (confidence → ∞)
    float variance_gamma;  // clip AABB half-width scale (× σ)
    float _pad;
};

// binding 6: the camera/extent UNIFORM block — byte-identical field layout to the marcher /
// resolve / SSAO / à-trous / shadow-temporal `Camera` (see `ray_gen.hlsli`'s doc: "Both
// including TUs declare the SAME 80-byte cbuffer Camera"). NOT necessarily unjittered as of C1:
// `boyko_render::upload_camera_ring_sheared` writes this SAME ring slot with `cam_forward`
// SHEARED by the per-pixel jitter offset whenever `TaaConfig::jitter_scope == RasterAndBasis` is
// armed, so this shader's `generate_ray` may sample a jittered sub-pixel position — matching
// every other consumer of this ring on that frame. The C2 differential reprojection below does
// not depend on this basis being unjittered (see the module doc).
cbuffer Camera : register(b6) {
    uint   count;        // total pixel count = img_w * img_h
    uint   img_w_raw;    // runtime extent width  (0 => IMG_W_DEFAULT)
    uint   img_h_raw;    // runtime extent height (0 => IMG_H_DEFAULT)
    uint   camera_mode;  // RAYGEN_CAM_ORTHO | RAYGEN_CAM_PERSPECTIVE (see ray_gen.hlsli)
    float4 cam_eye;
    float4 cam_forward;  // xyz = forward basis, w = tan(fovY/2)
    float4 cam_right;    // xyz = right basis,   w = aspect (W/H)
    float4 cam_up;
};

// The shared camera ray-gen (VERBATIM include — the marcher / deferred-resolve precedent for
// binding-agnostic reuse: this header takes the resolved camera fields as plain parameters
// rather than reading a global cbuffer, so it composes after `cbuffer Camera` above).
#include "ray_gen.hlsli"

// binding 7: the `MotionCam` UBO — the current + last frame's marcher-aligned proj·view pair
// (`boyko_render::motion_cam::MotionCam::to_bytes`, 128 B, column-major std140 — the SAME
// convention `gbuffer_mrt.vs.hlsl`'s `MotionCam` cbuffer consumes via `mul(m, v)`). Both fields
// are built from `marcher_view_proj_rows` (`boyko_render::view`), which is ALWAYS called with
// zero jitter — independent of the b6 camera basis above, which C1's `RasterAndBasis` scope MAY
// shear. C2 reads BOTH `mc_cur_view_proj` and `mc_prev_view_proj` (the differential
// reprojection below); a future v1.1 per-object cross-check remains a candidate consumer too.
[[vk::binding(7, 0)]] cbuffer MotionCam : register(b7) {
    float4x4 mc_cur_view_proj;
    float4x4 mc_prev_view_proj;
};

// The host-forced reset flag (`boyko_render::taa_state::TaaState::advance`) — TRUE on TAA's
// first armed frame or a resize (the allocated-`taa_hist` shape changed, so the previous
// contents are meaningless). Sets `blend_factor == 1.0` unconditionally this dispatch.
struct TaaPush { uint reset; };
[[vk::push_constant]] TaaPush pc;

// The legacy 64×64 fixture extent when the UBO extent is zero (mirrors the marcher / resolve /
// SSAO / à-trous / shadow-temporal fallback).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// The background sentinel (mirrors the marcher's `gViewT` 1e30 / shadow-temporal's `VIEWT_BG`):
// a center pixel with no surface has no meaningful reprojection ray, so it always resets.
static const float VIEWT_BG = 1.0e30;

// The confidence cap: bounding `1.0 / confidence` at `min_blend` (beyond this the adaptive
// factor would clamp there anyway, so capping the counter avoids unbounded SFLOAT growth).
static const float CONFIDENCE_CAP = 256.0;

// Marcher-aligned clip → [0,1]^2 screen UV — VERBATIM the `gbuffer_mrt.fs.hlsl` `clip_to_uv`
// formula (the projection already bakes the y-flip into clip.y, so this is the plain NDC remap,
// NO extra negation). Sharing the exact formula is what makes a static camera's `MV ≡ 0` proof
// hold bit-for-bit against the SAME convention the raster MV path would use.
float2 clip_to_uv(float4 clip) {
    return (clip.xy / clip.w) * 0.5 + 0.5;
}

// 1D Catmull-Rom basis weights (uniform, tension 0.5) at fractional offset `t` in `[0, 1)`,
// returning `{w(-1), w(0), w(1), w(2)}` for the four taps straddling the sample point.
float4 catmull_rom_weights(float t) {
    float t2 = t * t;
    float t3 = t2 * t;
    float w0 = -0.5 * t3 + 1.0 * t2 - 0.5 * t;
    float w1 = 1.5 * t3 - 2.5 * t2 + 1.0;
    float w2 = -1.5 * t3 + 2.0 * t2 + 0.5 * t;
    float w3 = 0.5 * t3 - 0.5 * t2;
    return float4(w0, w1, w2, w3);
}

// The 16-tap separable bicubic Catmull-Rom reconstruction of `gHistIn.rgb` at fractional UV
// `uv` — see the module doc for why this is `Load`-based (no hardware sampler on the GENERAL-
// layout history ring) rather than the literature's 5-tap bilinear-reduced variant. Edge texels
// clamp (never wrap), matching every other neighborhood tap in this codebase.
float3 sample_history_catmull_rom(float2 uv, uint w, uint h) {
    float2 sp = uv * float2(w, h) - 0.5;
    float2 ipos = floor(sp);
    float2 frac = sp - ipos;
    float4 wx = catmull_rom_weights(frac.x);
    float4 wy = catmull_rom_weights(frac.y);
    int2 max_tc = int2((int)w - 1, (int)h - 1);
    float3 sum = float3(0.0, 0.0, 0.0);
    float wsum = 0.0;
    [unroll]
    for (int j = -1; j <= 2; ++j) {
        float wyj = (j == -1) ? wy.x : (j == 0) ? wy.y : (j == 1) ? wy.z : wy.w;
        [unroll]
        for (int i = -1; i <= 2; ++i) {
            float wxi = (i == -1) ? wx.x : (i == 0) ? wx.y : (i == 1) ? wx.z : wx.w;
            int2 tc = clamp(int2(ipos) + int2(i, j), int2(0, 0), max_tc);
            float weight = wxi * wyj;
            sum += gHistIn.Load(tc).rgb * weight;
            wsum += weight;
        }
    }
    // The Catmull-Rom kernel sums to 1.0 analytically; the `max` guards clamped-edge drift
    // (a boundary pixel's taps fold onto fewer distinct texels, not a numerically zero sum).
    return sum / max(wsum, 1e-5);
}

// Clips `color` TOWARD the AABB center `[aabb_min, aabb_max]` (Karis/Lottes' anti-ghosting
// clip — NOT a per-channel clamp, which would shift hue): if `color` is outside the box, it is
// pulled back along the ray from the box center THROUGH `color` to the box boundary, preserving
// the color's direction (hue/saturation) while bounding its magnitude to the current frame's
// local neighborhood.
float3 clip_toward_aabb_center(float3 color, float3 aabb_min, float3 aabb_max) {
    float3 p_clip = 0.5 * (aabb_max + aabb_min);
    float3 e_clip = 0.5 * (aabb_max - aabb_min) + 1e-6;
    float3 v_clip = color - p_clip;
    float3 v_unit = v_clip / e_clip;
    float3 a_unit = abs(v_unit);
    float ma_unit = max(a_unit.x, max(a_unit.y, a_unit.z));
    return (ma_unit > 1.0) ? (p_clip + v_clip / ma_unit) : color;
}

// Rec. 709 relative luma — the Karis inverse-tonemap firefly-suppression weight
// (`w = 1 / (1 + luma)`), applied symmetrically to both blend operands then undone after the
// lerp, so a single bright outlier tap cannot dominate the accumulated average.
float luma(float3 c) {
    return dot(c, float3(0.2126, 0.7152, 0.0722));
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
    int3 coord3 = int3((int)px, (int)py, 0);
    int2 coord = int2((int)px, (int)py);

    float3 cur_lit = gLit.Load(coord3).rgb;
    float view_t = gViewT.Load(coord);
    bool has_surface = view_t < VIEWT_BG;

    // --- W3/C2 camera-only MV reconstruction (differential — jitter-free by construction) ----
    // The view ray for this pixel under the shader's OWN (possibly C1-sheared) camera basis —
    // used for BOTH the surface-point reprojection and the background point-at-infinity
    // reprojection below.
    float3 ro, rd;
    generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);

    // This pixel's own UNJITTERED screen-space UV (pixel center) — the history grid's own
    // addressing, and the reprojection target when nothing moved.
    float2 pixel_uv = (float2(px, py) + 0.5) / float2(w, h);

    // W5-FIX (static-scene silhouette supersampling): a BACKGROUND pixel (`!has_surface`) MUST
    // still accumulate — it is NOT a disocclusion. Under sub-pixel jitter a silhouette pixel FLIPS
    // between mesh-covered and background across the Halton cycle; the previous code treated every
    // uncovered frame as `reset_now` (full replace), which WIPED the accumulated color to the bare
    // current sample, so an edge pixel only ever held the latest HARD state (mesh OR sky) — a
    // shifted staircase, never the partial-coverage average that IS the anti-aliasing.
    //
    // A background pixel has no finite surface point, so it reprojects the POINT AT INFINITY along
    // the view ray `rd` (a direction, `w = 0`): `mul(view_proj, float4(rd, 0))`. This is
    // rotation-correct AND translation-invariant — an infinitely-far sky point does not move under
    // camera translation and moves correctly under rotation (the far-plane background MV Bevy uses),
    // so a TEXTURED skybox tracks without smear, not just a flat sky.
    //
    // C2 (jitter-free fix): `mc_cur_view_proj`/`mc_prev_view_proj` are ALWAYS built with zero
    // jitter (see the module doc), independent of `cam_forward` above, which C1's `RasterAndBasis`
    // scope MAY shear. Reprojecting `P` through `mc_prev_view_proj` alone therefore no longer
    // collapses to `pixel_uv` for a static camera whenever the basis is sheared — `P` sits on a
    // sheared ray, so `proj(P)` lands off `pixel_uv` by the shear itself, wobbling every frame
    // with the Halton phase. The fix reprojects `P` through BOTH `cur` and `prev`, and displaces
    // `pixel_uv` by the DELTA between them rather than using the raw `prev`-projected UV:
    //
    //   history_uv = pixel_uv + (uv(prev_clip) - uv(cur_clip))
    //
    // For a STATIC camera `mc_prev_view_proj == mc_cur_view_proj` bit-for-bit, so the delta is
    // two evaluations of the IDENTICAL matrix against the IDENTICAL `P` — exactly zero — and
    // `history_uv == pixel_uv` BY CONSTRUCTION, independent of whether the ray that produced `P`
    // was sheared. This also cancels any common-mode floating-point error in the camera-basis
    // reconstruction, so it is strictly more accurate than the pre-C2 code even when the basis is
    // NOT sheared (`RasterOnly` / jitter disarmed). Only a genuine disocclusion (a reprojection
    // that lands OFF-screen / behind the camera) or the host-forced reset (first armed frame /
    // resize) still forces the single-frame replace.
    float4 cur_clip, prev_clip;
    if (has_surface) {
        float3 P = ro + rd * view_t;
        cur_clip = mul(mc_cur_view_proj, float4(P, 1.0));
        prev_clip = mul(mc_prev_view_proj, float4(P, 1.0));
    } else {
        cur_clip = mul(mc_cur_view_proj, float4(rd, 0.0));
        prev_clip = mul(mc_prev_view_proj, float4(rd, 0.0));
    }
    float2 history_uv = pixel_uv + (clip_to_uv(prev_clip) - clip_to_uv(cur_clip));
    // Both `cur_clip.w` and `prev_clip.w` gate the divide `clip_to_uv` performs above: a
    // behind-camera `cur_clip.w <= 0.0` would make `clip_to_uv(cur_clip)` itself degenerate
    // (dividing by a non-positive `w`), corrupting the delta the SAME way a degenerate
    // `prev_clip.w` would corrupt `prev_uv` in the pre-C2 code — so both sides of the
    // differential need the guard, not just the historical `prev` side.
    bool off_screen =
        any(history_uv < 0.0) || any(history_uv > 1.0) || prev_clip.w <= 0.0 || cur_clip.w <= 0.0;

    bool reset_now = (pc.reset != 0u) || off_screen;

    if (reset_now) {
        // Disocclusion / off-screen / host-forced reset: the I5-style single-frame fallback —
        // `blend_factor == 1.0` (full replace), confidence seeded to 1.0. No NaN risk: `cur_lit`
        // is the resolve's own finite LDR output, un-combined with any division here.
        gHistOut[coord] = float4(cur_lit, 1.0);
        gAaOut[coord] = float4(cur_lit, 1.0);
        return;
    }

    // --- 3×3 neighborhood moments of the CURRENT `lit` (the variance-clip AABB) -------------
    float3 mean = float3(0.0, 0.0, 0.0);
    float3 m2 = float3(0.0, 0.0, 0.0);
    [unroll]
    for (int oy = -1; oy <= 1; ++oy) {
        [unroll]
        for (int ox = -1; ox <= 1; ++ox) {
            int tx = clamp((int)px + ox, 0, (int)w - 1);
            int ty = clamp((int)py + oy, 0, (int)h - 1);
            float3 c = gLit.Load(int3(tx, ty, 0)).rgb;
            mean += c;
            m2 += c * c;
        }
    }
    mean /= 9.0;
    float3 variance = max(m2 / 9.0 - mean * mean, 0.0);
    float3 sigma = sqrt(variance);
    float3 aabb_min = mean - variance_gamma * sigma;
    float3 aabb_max = mean + variance_gamma * sigma;

    // --- History reproject + clip -------------------------------------------------------------
    float3 hist_raw = sample_history_catmull_rom(history_uv, w, h);
    float3 hist_clipped = clip_toward_aabb_center(hist_raw, aabb_min, aabb_max);

    // Nearest-tap confidence fetch (mirrors shadow_temporal's "bilinear color + nearest
    // metadata" split — blending a frame-count-like quantity would corrupt its meaning).
    int2 nearest_tc = clamp(
        int2(round(history_uv * float2(w, h) - 0.5)),
        int2(0, 0),
        int2((int)w - 1, (int)h - 1)
    );
    float conf_prev = gHistIn.Load(nearest_tc).a;
    float confidence = min(conf_prev + 1.0, CONFIDENCE_CAP);

    // --- Confidence-adaptive, luma-weighted blend ---------------------------------------------
    float blend_factor = clamp(1.0 / confidence, min_blend, default_blend);
    float3 w_cur = cur_lit / (1.0 + luma(cur_lit));
    float3 w_hist = hist_clipped / (1.0 + luma(hist_clipped));
    float3 blended_w = lerp(w_hist, w_cur, blend_factor);
    float3 out_color = blended_w / max(1.0 - luma(blended_w), 1e-4);

    gHistOut[coord] = float4(out_color, confidence);
    gAaOut[coord] = float4(out_color, 1.0);
}
