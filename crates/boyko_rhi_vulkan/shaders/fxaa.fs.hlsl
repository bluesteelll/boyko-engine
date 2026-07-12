// Anti-Aliasing campaign, Stage 1 — FXAA (Fast Approximate Anti-Aliasing) post-process
// fragment shader. Reads the deferred resolve's LIT color as a bilinear combined-image-
// sampler and writes an edge-antialiased color into the `aa_out` target; the present-blit
// then samples `aa_out` instead of `lit` (AA on) — a byte-identical no-op when AA is off
// (the pass is not recorded at all).
//
// Algorithm: Timothy Lottes' FXAA 3.11 luma-edge AA, in the compact form popularised by
// Simon Rodriguez's "Implementing FXAA" reference — luma edge detection, horizontal/vertical
// edge orientation, an edge-end search along the edge, and a sub-pixel aliasing term. FXAA is
// designed to run on a NON-LINEAR (display / gamma-space) image: the LIT target is the
// post-tonemap, OETF-encoded resolve output, which is exactly that input.
//
// INTERFACE (must match the host FxaaActivation wiring):
//   set 0, binding 0 : LIT color, a COMBINED_IMAGE_SAMPLER — the sampler MUST be LINEAR
//                      (bilinear) filtering; FXAA's final sub-texel tap relies on it.
//   push constant    : { float2 rcp_frame; } = (1/width, 1/height) of the LIT extent.
// Vertex shader: reuse fullscreen_sample.vs.hlsl (the SV_VertexID fullscreen triangle);
// its interpolated UV maps 1:1 to the LIT texels (Vulkan framebuffer Y-down, self-consistent
// with how LIT was written).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time), .spv hand-committed:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 fxaa.fs.hlsl -Fo fxaa.fs.spv

[[vk::binding(0, 0)]] Texture2D    g_tex : register(t0);
[[vk::binding(0, 0)]] SamplerState g_smp : register(s0);

struct FxaaPush {
    float2 rcp_frame; // (1/width, 1/height) of the LIT extent.
};
[[vk::push_constant]] FxaaPush pc;

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

// --- Quality knobs (FXAA 3.11 "PC extreme quality" preset, 12 taps) -----------------------
// The edge is only treated when the local contrast exceeds a floor; the floor scales with the
// brighter luma so dark regions are not over-smoothed. Raised from Lottes' "High" tier
// (0.0312 / 0.125) to his "Ultra/Extreme" tier (0.0156 / 0.063) so fainter silhouette steps
// are also treated — on the deferred gamma `lit` target the sphere-vs-sky contrast is modest,
// so the "High" floor left most of the staircase below the edge gate (owner: "almost no
// effect"). SUBPIXEL_QUALITY raised 0.75 -> 1.0 (max sub-pixel term): FXAA is the CHEAP
// fallback (TAA is the temporal path), so we want the strongest per-frame smoothing FXAA can
// give; the interiors of the smooth-shaded sphere bodies carry no high-frequency luma, so the
// sub-pixel low-pass does not over-blur them (visually verified on the 5-sphere scene).
static const float EDGE_THRESHOLD_MIN = 0.0156;
static const float EDGE_THRESHOLD_MAX = 0.063;
static const float SUBPIXEL_QUALITY   = 1.0;
static const int   ITERATIONS         = 12;

// Per-iteration step multipliers along the edge (FXAA_QUALITY preset): small near the pixel,
// growing so a long smooth edge is still terminated within ITERATIONS taps.
static float quality_step(int i) {
    // {1,1,1,1,1, 1.5, 2,2,2,2, 4, 8}
    if (i < 5) return 1.0;
    if (i == 5) return 1.5;
    if (i < 10) return 2.0;
    if (i == 10) return 4.0;
    return 8.0;
}

// Perceptual luma of a display-space color (sqrt approximates the eye's response, matching the
// FXAA reference). Green-dominant Rec.601-ish weights.
static float rgb2luma(float3 rgb) {
    return sqrt(dot(rgb, float3(0.299, 0.587, 0.114)));
}

float4 main(VsOut input) : SV_Target0 {
    float2 uv = input.uv;
    float3 color_center = g_tex.SampleLevel(g_smp, uv, 0.0).rgb;

    // Luma at the center and the four direct neighbours.
    float luma_center = rgb2luma(color_center);
    float luma_down  = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(0, 1)).rgb);
    float luma_up    = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(0, -1)).rgb);
    float luma_left  = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(-1, 0)).rgb);
    float luma_right = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(1, 0)).rgb);

    float luma_min = min(luma_center, min(min(luma_down, luma_up), min(luma_left, luma_right)));
    float luma_max = max(luma_center, max(max(luma_down, luma_up), max(luma_left, luma_right)));
    float luma_range = luma_max - luma_min;

    // Not an edge (flat / below the contrast floor): return the un-touched center — this makes
    // FXAA a near-no-op on smooth interiors, only working the silhouettes/high-contrast edges.
    if (luma_range < max(EDGE_THRESHOLD_MIN, luma_max * EDGE_THRESHOLD_MAX)) {
        return float4(color_center, 1.0);
    }

    // The four diagonal neighbours (for edge orientation + the sub-pixel term).
    float luma_dl = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(-1, 1)).rgb);
    float luma_ur = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(1, -1)).rgb);
    float luma_ul = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(-1, -1)).rgb);
    float luma_dr = rgb2luma(g_tex.SampleLevel(g_smp, uv, 0.0, int2(1, 1)).rgb);

    float luma_down_up    = luma_down + luma_up;
    float luma_left_right = luma_left + luma_right;
    float luma_left_corners  = luma_dl + luma_ul;
    float luma_down_corners  = luma_dl + luma_dr;
    float luma_right_corners = luma_dr + luma_ur;
    float luma_up_corners    = luma_ul + luma_ur;

    // Horizontal vs vertical edge estimator (|gradient| across each axis).
    float edge_horizontal =
        abs(-2.0 * luma_left + luma_left_corners) +
        abs(-2.0 * luma_center + luma_down_up) * 2.0 +
        abs(-2.0 * luma_right + luma_right_corners);
    float edge_vertical =
        abs(-2.0 * luma_up + luma_up_corners) +
        abs(-2.0 * luma_center + luma_left_right) * 2.0 +
        abs(-2.0 * luma_down + luma_down_corners);

    bool is_horizontal = (edge_horizontal >= edge_vertical);

    // The two lumas on either side of the edge, and the local gradients.
    float luma1 = is_horizontal ? luma_down : luma_left;
    float luma2 = is_horizontal ? luma_up : luma_right;
    float gradient1 = luma1 - luma_center;
    float gradient2 = luma2 - luma_center;

    bool is1_steepest = abs(gradient1) >= abs(gradient2);
    float gradient_scaled = 0.25 * max(abs(gradient1), abs(gradient2));

    // Step one texel toward the edge; seed the average luma on that side.
    float step_length = is_horizontal ? pc.rcp_frame.y : pc.rcp_frame.x;
    float luma_local_average;
    if (is1_steepest) {
        step_length = -step_length;
        luma_local_average = 0.5 * (luma1 + luma_center);
    } else {
        luma_local_average = 0.5 * (luma2 + luma_center);
    }

    float2 current_uv = uv;
    if (is_horizontal) {
        current_uv.y += step_length * 0.5;
    } else {
        current_uv.x += step_length * 0.5;
    }

    // March along the edge in both directions until each end (where the luma leaves the edge band).
    float2 offset = is_horizontal ? float2(pc.rcp_frame.x, 0.0) : float2(0.0, pc.rcp_frame.y);
    float2 uv1 = current_uv - offset;
    float2 uv2 = current_uv + offset;

    float luma_end1 = rgb2luma(g_tex.SampleLevel(g_smp, uv1, 0.0).rgb) - luma_local_average;
    float luma_end2 = rgb2luma(g_tex.SampleLevel(g_smp, uv2, 0.0).rgb) - luma_local_average;

    bool reached1 = abs(luma_end1) >= gradient_scaled;
    bool reached2 = abs(luma_end2) >= gradient_scaled;
    bool reached_both = reached1 && reached2;

    if (!reached1) uv1 -= offset;
    if (!reached2) uv2 += offset;

    if (!reached_both) {
        [loop]
        for (int i = 2; i < ITERATIONS; i++) {
            if (!reached1) {
                luma_end1 = rgb2luma(g_tex.SampleLevel(g_smp, uv1, 0.0).rgb) - luma_local_average;
                reached1 = abs(luma_end1) >= gradient_scaled;
            }
            if (!reached2) {
                luma_end2 = rgb2luma(g_tex.SampleLevel(g_smp, uv2, 0.0).rgb) - luma_local_average;
                reached2 = abs(luma_end2) >= gradient_scaled;
            }
            float qstep = quality_step(i);
            if (!reached1) uv1 -= offset * qstep;
            if (!reached2) uv2 += offset * qstep;
            reached_both = reached1 && reached2;
            if (reached_both) break;
        }
    }

    // Distances to each edge end; pick the closer end to weight the offset.
    float distance1 = is_horizontal ? (uv.x - uv1.x) : (uv.y - uv1.y);
    float distance2 = is_horizontal ? (uv2.x - uv.x) : (uv2.y - uv.y);

    bool is_direction1 = distance1 < distance2;
    float distance_final = min(distance1, distance2);
    float edge_thickness = (distance1 + distance2);
    float pixel_offset = -distance_final / edge_thickness + 0.5;

    // Only offset if the center luma is on the correct (edge) side; otherwise stay put.
    bool is_luma_center_smaller = luma_center < luma_local_average;
    bool correct_variation =
        ((is_direction1 ? luma_end1 : luma_end2) < 0.0) != is_luma_center_smaller;
    float final_offset = correct_variation ? pixel_offset : 0.0;

    // Sub-pixel aliasing term: a low-pass estimate of local luma vs the center, weighted and
    // clamped, taken as an alternative offset (the larger of the two wins).
    float luma_average = (1.0 / 12.0) * (2.0 * (luma_down_up + luma_left_right)
        + luma_left_corners + luma_right_corners);
    float subpixel_offset1 = clamp(abs(luma_average - luma_center) / luma_range, 0.0, 1.0);
    float subpixel_offset2 = (-2.0 * subpixel_offset1 + 3.0) * subpixel_offset1 * subpixel_offset1;
    float subpixel_offset_final = subpixel_offset2 * subpixel_offset2 * SUBPIXEL_QUALITY;

    final_offset = max(final_offset, subpixel_offset_final);

    // Final sub-texel tap: shift the UV perpendicular to the edge by the computed offset.
    float2 final_uv = uv;
    if (is_horizontal) {
        final_uv.y += final_offset * step_length;
    } else {
        final_uv.x += final_offset * step_length;
    }

    return float4(g_tex.SampleLevel(g_smp, final_uv, 0.0).rgb, 1.0);
}
