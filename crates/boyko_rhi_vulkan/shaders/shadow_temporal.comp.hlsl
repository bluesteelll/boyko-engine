// Render Shadow Rung 3b — the TEMPORAL shadow-visibility reproject + accumulate (Option B).
//
// ONE dispatch, AFTER the à-trous spatial filter (if any) and BEFORE the RESOLVE_DENOISED resolve.
// It reprojects the current shadow-visibility (`gVisIn` — the à-trous output in `mode == Both`, or
// the raw VIS output in `mode == Temporal`) through the per-pixel motion vector into a per-FIF
// history ring, variance-clamps the reprojected history against the current 3×3 neighbourhood
// (Salvi's scalar clamp — the cheapest robust ghosting suppressor), blends with a velocity-adaptive
// feedback `k`, and resets hard to the current single-frame value on disocclusion (off-screen /
// unconfident history / a prev-vs-current depth swap). The accumulated visibility is written to BOTH
// the history ring (for the next frame) AND `gTemporalOut` (the RESOLVE_DENOISED reads it at its
// `gShadowVis` @21 slot, exactly as it read the à-trous output in Rung 3a).
//
// # The history ring (R16G16B16A16_UNORM, GENERAL, per-FIF, cross-frame seeded)
//
//   R = accumulated visibility [0,1]
//   G = confidence / frame-count, stored NORMALIZED (conf / CONF_MAX) so it fits UNORM
//   B = the surface depth this pixel was accumulated at, stored NORMALIZED (view_t / DEPTH_NORM,
//       saturated so the background sentinel pins to 1.0) — W2: carrying prev depth is what makes a
//       same-pixel surface swap (a moving box sliding over the floor) DETECTABLE as a disocclusion.
//   A = reserved (0)
//
// Frame `fi` READS `hist[1-fi]` (`gHistIn`) and WRITES `hist[fi]` (`gHistOut`). The ring is seeded
// GENERAL (the DDGI `seeded_readers_at_layout` precedent) so the very first read is a defined
// (zeroed ⇒ conf==0 ⇒ reset = I5 single-frame fallback) texel, never UNDEFINED.
//
// # Correctness under motion (the #1 sensitivity — see the plan's Decision 7)
//
//   1. TRUE per-object mesh MV (the raster 5a variant) reprojects moving boxes to where they WERE.
//   2. Variance clamp — any residual MV error (fp16; SDF pixels are camera-only) is clamped to the
//      current 3×3 AABB, so a wrong reprojection pulls toward a valid neighbour, never a double-image.
//   3. Velocity-k + disocclusion reset — `k`↓ under motion; a hard reset (→ the current single-frame
//      value, I5) on off-screen / conf==0 / prev-vs-cur depth mismatch.
// The ONE uncovered case is a moving SDF BODY (SDF pixels reproject camera-only) — bounded by 2+3,
// surfaced + deferred, gated behind the in-motion owner-eval.
//
// # Byte-identity
//
// This shader is NEVER dispatched unless the temporal denoiser is active (`mode ∈ {Temporal, Both}`);
// `mode ∈ {None, Spatial}` names no temporal ResId, so the golden path is untouched. It is a NEW
// `.spv` (no base variant to freeze).
//
// Compiled offline (hermetic — no SDK at `cargo build` time) with:
//   dxc -spirv -T cs_6_0 -E main "-fspv-target-env=vulkan1.3" \
//       shadow_temporal.comp.hlsl -Fo shadow_temporal.comp.spv

// bindings 0..2 (READ): the current shadow-vis (R=vis, G=validity — the à-trous/VIS output, RG16
// UNORM), the per-pixel motion vector (RG16 SFLOAT Δuv, `uv_prev − uv_cur`), and the surface depth
// (`gViewT`, r32f — the marcher's ray param `t`; 1e30 on background).
[[vk::image_format("rg16")]]  RWTexture2D<float2> gVisIn     : register(u0);
[[vk::image_format("rg16f")]] RWTexture2D<float2> gMotionVec : register(u1);
[[vk::image_format("r32f")]]  RWTexture2D<float>  gViewT     : register(u2);
// bindings 3..4: the history ring — READ `hist[1-fi]` (bilinear vis + nearest conf/depth) + WRITE
// `hist[fi]`. Both RGBA16 UNORM (the uniform pin matches whichever ring slot is bound each parity).
[[vk::image_format("rgba16")]] RWTexture2D<float4> gHistIn  : register(u3);
[[vk::image_format("rgba16")]] RWTexture2D<float4> gHistOut : register(u4);
// binding 5 (WRITE): the temporal-out the RESOLVE_DENOISED reads at `gShadowVis` @21 (RG16 UNORM,
// R=accumulated vis, G=validity=1). A DEDICATED target (not an in-place `gVisIn` write-back) so the
// 3×3 neighbourhood read cannot race the accumulate write.
[[vk::image_format("rg16")]] RWTexture2D<float2> gTemporalOut : register(u5);

// binding 6: the temporal tunables UBO — byte-mirrors `boyko_render::ResolvedTemporalShadow` (16 B,
// std140: four f32 = one vec4 slot). Owner-retunable each frame (live). `feedback_max` is the
// steady-state history weight (a static camera converges to it); `feedback_min` is the weight under
// fast motion (more of the current single frame); `variance_gamma` scales the clamp AABB half-width;
// `depth_tol` is the relative prev-vs-cur depth tolerance τ for the disocclusion reset.
cbuffer ResolvedTemporalShadow : register(b6) {
    float feedback_max;   // steady-state history weight (static camera)
    float feedback_min;   // history weight under fast motion
    float variance_gamma; // clamp AABB half-width scale (× σ)
    float depth_tol;      // τ: relative prev-vs-cur depth reset threshold
};

// binding 7: the camera/extent UNIFORM block — byte-identical field layout to the marcher / resolve
// / SSAO / à-trous `Camera`. Only the extent (`img_w`/`img_h`) is consumed here (the reproject works
// in UV/pixel space; no ray-gen needed).
cbuffer Camera : register(b7) {
    uint   count;        // total pixel count = img_w * img_h
    uint   img_w_raw;    // runtime extent width  (0 => IMG_W_DEFAULT)
    uint   img_h_raw;    // runtime extent height (0 => IMG_H_DEFAULT)
    uint   camera_mode;  // unused here (kept for layout parity)
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};

// The legacy 64×64 fixture extent when the UBO extent is zero (mirrors the marcher/resolve/SSAO/à-trous).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// The confidence cap (frame-count ceiling): the accumulated sample count saturates here, bounding the
// steady-state history weight so a stale region still refreshes at ~1/CONF_MAX per frame. A shader
// CONSTANT (not the UBO — it sets the [0,1] UNORM quantization of the G channel, not a live knob).
static const float CONF_MAX = 32.0;
// The velocity (in PIXELS of |Δuv|·extent) at which `k` reaches `feedback_min`. Faster motion ⇒ less
// history (the clamp + reset carry correctness; `k` only sets how fast we let go). Static const.
static const float VELOCITY_REF = 16.0;
// The depth normalizer: `view_t` (world ray param, ≤ ~64 for far mesh, ≤ ~10 for SDF) mapped into the
// UNORM B channel. `min(view_t, DEPTH_NORM)/DEPTH_NORM` saturates the 1e30 background sentinel to 1.0,
// so a background⇄foreground swap always trips the depth disocclusion. Mirrors `MESH_DEPTH_T_MAX`.
static const float DEPTH_NORM = 64.0;
// The background sentinel (mirror the marcher's gViewT 1e30): a center pixel with no surface passes
// the neutral vis through (reset path), never accumulating garbage.
static const float VIEWT_BG = 1.0e30;

// Normalize a raw `view_t` into the [0,1] UNORM depth channel (saturating the background sentinel).
float depth_norm(float view_t) {
    return min(view_t, DEPTH_NORM) / DEPTH_NORM;
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

    // --- Current sample ------------------------------------------------------------------------
    float2 cur = gVisIn.Load(coord);
    float  cur_vis = cur.r;          // current (spatially-filtered) visibility
    float  cur_valid = cur.g;        // 1 = a real mesh-shadow sample, 0 = neutral fill / background
    float  view_t_c = gViewT.Load(coord);
    float  cur_depth = depth_norm(view_t_c);

    // A pixel with no real shadow sample (background / non-directional / non-lit) carries the neutral
    // full-vis; pass it straight through (no accumulation) + seed the history unconfident. This is the
    // I5 fallback for the trivial case + keeps the history's confidence honest for later reprojection.
    if (cur_valid < 0.5 || view_t_c >= VIEWT_BG) {
        gHistOut[uint2(px, py)] = float4(cur_vis, 0.0, cur_depth, 0.0);
        gTemporalOut[uint2(px, py)] = float2(cur_vis, cur_valid);
        return;
    }

    // --- 3×3 neighbourhood moments of the CURRENT vis (the variance-clamp AABB) ----------------
    // Salvi's scalar clamp: reproject-then-clamp the history into [μ − γσ, μ + γσ] of the current
    // neighbourhood, so a mis-reprojected history sample is pulled toward a valid local value (the
    // ghosting ceiling). Only valid taps contribute; a hard-edge island (no valid neighbour) degrades
    // to the center value (zero-width clamp), i.e. the current single frame.
    float sum = 0.0;
    float sum_sq = 0.0;
    float n = 0.0;
    [unroll]
    for (int oy = -1; oy <= 1; ++oy) {
        [unroll]
        for (int ox = -1; ox <= 1; ++ox) {
            int tx = clamp((int)px + ox, 0, (int)w - 1);
            int ty = clamp((int)py + oy, 0, (int)h - 1);
            float2 t = gVisIn.Load(int2(tx, ty));
            // Gate dead taps (validity 0) out of the moments so background/neutral pixels don't widen
            // the AABB toward the neutral 1.0.
            float vt = t.r;
            float m = t.g >= 0.5 ? 1.0 : 0.0;
            sum += vt * m;
            sum_sq += vt * vt * m;
            n += m;
        }
    }
    // At least the center tap is valid here (cur_valid >= 0.5), so n >= 1.
    float mean = sum / n;
    float var = max(sum_sq / n - mean * mean, 0.0);
    float sigma = sqrt(var);
    float lo = mean - variance_gamma * sigma;
    float hi = mean + variance_gamma * sigma;

    // --- Reproject through the motion vector ---------------------------------------------------
    float2 duv = gMotionVec.Load(coord);                       // uv_prev − uv_cur
    float2 pixel_uv = (float2(px, py) + 0.5) / float2(w, h);
    float2 prev_uv = pixel_uv + duv;                           // where this surface was last frame

    // Off-screen ⇒ no history ⇒ reset (I5 single-frame).
    bool off_screen = any(prev_uv < 0.0) || any(prev_uv > 1.0);

    // --- Sample the history: bilinear vis + nearest conf/depth ---------------------------------
    // Bilinear on the accumulated visibility (smooth reprojection); NEAREST on conf/depth (blending
    // those across a surface boundary would corrupt the disocclusion test). The 4 corner texels are
    // loaded once; the nearest is selected from them (no 5th fetch).
    float2 sp = prev_uv * float2(w, h) - 0.5;                  // pixel-space sample center
    float2 bf = frac(sp);
    int2 b0 = int2(floor(sp));
    int2 c00 = clamp(b0 + int2(0, 0), int2(0, 0), int2((int)w - 1, (int)h - 1));
    int2 c10 = clamp(b0 + int2(1, 0), int2(0, 0), int2((int)w - 1, (int)h - 1));
    int2 c01 = clamp(b0 + int2(0, 1), int2(0, 0), int2((int)w - 1, (int)h - 1));
    int2 c11 = clamp(b0 + int2(1, 1), int2(0, 0), int2((int)w - 1, (int)h - 1));
    float4 h00 = gHistIn.Load(c00);
    float4 h10 = gHistIn.Load(c10);
    float4 h01 = gHistIn.Load(c01);
    float4 h11 = gHistIn.Load(c11);
    float vis_hist = lerp(lerp(h00.r, h10.r, bf.x), lerp(h01.r, h11.r, bf.x), bf.y);
    // The nearest corner (for conf + depth).
    float4 hn = (bf.x < 0.5)
        ? ((bf.y < 0.5) ? h00 : h01)
        : ((bf.y < 0.5) ? h10 : h11);
    float conf_prev = hn.g * CONF_MAX;
    float depth_hist = hn.b;

    // --- Disocclusion (W2) ---------------------------------------------------------------------
    // Reset to the current single frame on ANY of: off-screen; unconfident history (conf == 0, the
    // seed / a prior reset); a prev-vs-current depth swap (`|depth_hist − cur_depth| > τ·cur_depth`
    // — the same-pixel surface-swap the moving box triggers, now detectable via the history B lane).
    bool no_conf = conf_prev < 0.5;
    bool depth_swap = abs(depth_hist - cur_depth) > depth_tol * cur_depth;
    bool reset = off_screen || no_conf || depth_swap;

    float out_vis;
    float out_conf;
    if (reset) {
        out_vis = cur_vis;
        out_conf = 1.0;
    } else {
        // Clamp the reprojected history to the current neighbourhood AABB (ghosting ceiling), then
        // blend with a velocity-adaptive feedback: `k` = feedback_max at rest → feedback_min at
        // VELOCITY_REF pixels/frame (faster motion trusts the current frame more).
        float clamped_hist = clamp(vis_hist, lo, hi);
        float speed = length(duv * float2(w, h));
        float k = lerp(feedback_max, feedback_min, saturate(speed / VELOCITY_REF));
        out_vis = lerp(cur_vis, clamped_hist, k);
        out_conf = min(conf_prev + 1.0, CONF_MAX);
    }

    // Store the new history (conf normalized into the UNORM G lane) + the temporal-out the DENOISED
    // resolve reads. Exactly ONE texel each per pixel.
    gHistOut[uint2(px, py)] = float4(out_vis, out_conf / CONF_MAX, cur_depth, 0.0);
    gTemporalOut[uint2(px, py)] = float2(out_vis, 1.0);
}
