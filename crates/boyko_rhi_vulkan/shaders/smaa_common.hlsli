// SMAA 1x (AA campaign, Stage 2) — shared constants + helper functions, ported VERBATIM
// (function bodies unchanged) from the reference implementation:
//
//   Copyright (C) 2013 Jorge Jimenez, Jose I. Echevarria, Belen Masia, Fernando Navarro,
//   Diego Gutierrez. MIT license. https://github.com/iryoku/smaa
//
// PRESET_HIGH (diagonal + corner detection ON), SMAA_HLSL_4-equivalent MANUAL-DECODE
// diagonal search path (no Gather4 — Decision 5, vendor-risk avoidance on a fresh port).
//
// # API-layer adaptation (the ONLY thing changed vs the reference; algorithm bodies are a
// # verbatim transcription)
//
// - The reference's `SMAATexture2D(tex)` / `SMAATexturePass2D(tex)` porting macros are
//   replaced by explicit `Texture2D tex, SamplerState smp` parameter PAIRS: this port's
//   descriptor layout gives each bound texture its OWN `CombinedImageSampler` slot (edges@0,
//   areaTex@1, searchTex@2 in the weight pass; lit@0, weights@1 in the blend pass), even
//   though every slot's sampler is the SAME `VulkanSampler` object on the host side (one
//   LINEAR/ClampToEdge sampler for every SMAA tap — Open Q2). This is the minimal change
//   that lets each texture keep ITS OWN descriptor-bound sampler.
// - The reference's global `LinearSampler`/`PointSampler` pair collapses to the ONE shared
//   sampler passed in above: `SMAASamplePoint`/`SMAASample` both become `.Sample(smp, uv)`;
//   `SMAASampleLevelZero(Point)?` become `.SampleLevel(smp, uv, 0)` — the SAMPLE METHOD
//   (`.Sample` vs `.SampleLevel(...,0)`) is preserved per call site exactly as the reference
//   macro chose it (search loops use `.SampleLevel` to dodge derivative-in-non-uniform-flow
//   issues), only the SAMPLER OBJECT is unified.
// - `SMAA_RT_METRICS` (a compile-time macro in the reference) becomes the FRAGMENT-only
//   push-constant `SmaaPush.rt_metrics` below, threaded as an explicit parameter.
// - The reference's per-pass VERTEX SHADER offset outputs (`SMAAEdgeDetectionVS`,
//   `SMAABlendingWeightCalculationVS`, `SMAANeighborhoodBlendingVS`) are inlined as in-PS
//   helpers computed from `uv` + `rt_metrics` at the top of each entry shader (this port
//   reuses ONE shared `fullscreen_sample.vs` for all three passes, so there is no per-pass
//   VS to carry per-pass outputs) — bodies are a verbatim transcription of the VS math.
//
// `subsampleIndices` is always `float4(0,0,0,0)` at the call site (SMAA 1x — no temporal
// supersampling, see `@SUBSAMPLE_INDICES` in the reference header).

#ifndef SMAA_COMMON_HLSLI
#define SMAA_COMMON_HLSLI

// ---- PRESET_HIGH constants (verbatim) ---------------------------------------------------
static const float SMAA_THRESHOLD = 0.1;
static const int SMAA_MAX_SEARCH_STEPS = 16;
static const int SMAA_MAX_SEARCH_STEPS_DIAG = 8;
static const float SMAA_CORNER_ROUNDING = 25.0;
static const float SMAA_CORNER_ROUNDING_NORM = SMAA_CORNER_ROUNDING / 100.0;
static const float SMAA_LOCAL_CONTRAST_ADAPTATION_FACTOR = 2.0;

// ---- Non-configurable constants (verbatim) ----------------------------------------------
static const float SMAA_AREATEX_MAX_DISTANCE = 16.0;
static const float SMAA_AREATEX_MAX_DISTANCE_DIAG = 20.0;
static const float2 SMAA_AREATEX_PIXEL_SIZE = (1.0 / float2(160.0, 560.0));
static const float SMAA_AREATEX_SUBTEX_SIZE = (1.0 / 7.0);
// Deliberately NOT the physical packed size (64,16) — the reference's own comment explains
// this is the CROPPED logical size the search-length lookup maps into; do not "simplify".
static const float2 SMAA_SEARCHTEX_SIZE = float2(66.0, 33.0);
static const float2 SMAA_SEARCHTEX_PACKED_SIZE = float2(64.0, 16.0);

#define SMAA_AREATEX_SELECT(sample) sample.rg
#define SMAA_SEARCHTEX_SELECT(sample) sample.r

// ---- The FRAGMENT-only push constant (16 bytes) — the SMAA_RT_METRICS equivalent --------
struct SmaaPush {
    // (1/width, 1/height, width, height) of `present_extent` (the shared extent every SMAA
    // target is sized to).
    float4 rt_metrics;
};

// ---- Conditional move (verbatim SMAAMovc) -----------------------------------------------
void SMAAMovc(bool2 cond, inout float2 variable, float2 value) {
    if (cond.x) variable.x = value.x;
    if (cond.y) variable.y = value.y;
}

void SMAAMovc(bool4 cond, inout float4 variable, float4 value) {
    SMAAMovc(cond.xy, variable.xy, value.xy);
    SMAAMovc(cond.zw, variable.zw, value.zw);
}

// ---- Per-pass offset helpers (verbatim transcription of the reference VS bodies) --------

// SMAAEdgeDetectionVS, inlined: the west/top + left-left/top-top taps for pass 1.
void SMAAEdgeDetectionOffsets(float2 texcoord, float4 rt_metrics, out float4 offset[3]) {
    offset[0] = mad(rt_metrics.xyxy, float4(-1.0, 0.0, 0.0, -1.0), texcoord.xyxy);
    offset[1] = mad(rt_metrics.xyxy, float4( 1.0, 0.0, 0.0,  1.0), texcoord.xyxy);
    offset[2] = mad(rt_metrics.xyxy, float4(-2.0, 0.0, 0.0, -2.0), texcoord.xyxy);
}

// SMAABlendingWeightCalculationVS, inlined: the @PSEUDO_GATHER4 search-seed offsets +
// the search-loop end bounds for pass 2.
void SMAABlendingWeightOffsets(
    float2 texcoord,
    float4 rt_metrics,
    out float2 pixcoord,
    out float4 offset[3]
) {
    pixcoord = texcoord * rt_metrics.zw;

    offset[0] = mad(rt_metrics.xyxy, float4(-0.25, -0.125,  1.25, -0.125), texcoord.xyxy);
    offset[1] = mad(rt_metrics.xyxy, float4(-0.125, -0.25, -0.125,  1.25), texcoord.xyxy);

    offset[2] = mad(
        rt_metrics.xxyy,
        float4(-2.0, 2.0, -2.0, 2.0) * float(SMAA_MAX_SEARCH_STEPS),
        float4(offset[0].xz, offset[1].yw));
}

// SMAANeighborhoodBlendingVS, inlined: the bottom/right taps for pass 3.
void SMAANeighborhoodBlendingOffset(float2 texcoord, float4 rt_metrics, out float4 offset) {
    offset = mad(rt_metrics.xyxy, float4(1.0, 0.0, 0.0, 1.0), texcoord.xyxy);
}

// ---- Edge Detection Pixel Shader (First Pass) — Luma (verbatim SMAALumaEdgeDetectionPS,
// SMAA_PREDICATION off) ---------------------------------------------------------------------
float2 SMAALumaEdgeDetectionPS(
    float2 texcoord,
    float4 offset[3],
    Texture2D colorTex,
    SamplerState colorSmp
) {
    float2 threshold = float2(SMAA_THRESHOLD, SMAA_THRESHOLD);

    float3 weights = float3(0.2126, 0.7152, 0.0722);
    float L = dot(colorTex.Sample(colorSmp, texcoord).rgb, weights);

    float Lleft = dot(colorTex.Sample(colorSmp, offset[0].xy).rgb, weights);
    float Ltop  = dot(colorTex.Sample(colorSmp, offset[0].zw).rgb, weights);

    float4 delta;
    delta.xy = abs(L - float2(Lleft, Ltop));
    float2 edges = step(threshold, delta.xy);

    if (dot(edges, float2(1.0, 1.0)) == 0.0)
        discard;

    float Lright = dot(colorTex.Sample(colorSmp, offset[1].xy).rgb, weights);
    float Lbottom = dot(colorTex.Sample(colorSmp, offset[1].zw).rgb, weights);
    delta.zw = abs(L - float2(Lright, Lbottom));

    float2 maxDelta = max(delta.xy, delta.zw);

    float Lleftleft = dot(colorTex.Sample(colorSmp, offset[2].xy).rgb, weights);
    float Ltoptop = dot(colorTex.Sample(colorSmp, offset[2].zw).rgb, weights);
    delta.zw = abs(float2(Lleft, Ltop) - float2(Lleftleft, Ltoptop));

    maxDelta = max(maxDelta.xy, delta.zw);
    float finalDelta = max(maxDelta.x, maxDelta.y);

    edges.xy *= step(finalDelta, SMAA_LOCAL_CONTRAST_ADAPTATION_FACTOR * delta.xy);

    return edges;
}

// ---- Diagonal Search Functions (verbatim; diag detection is UNCONDITIONALLY on — PRESET_HIGH) --

float2 SMAADecodeDiagBilinearAccess(float2 e) {
    e.r = e.r * abs(5.0 * e.r - 5.0 * 0.75);
    return round(e);
}

float4 SMAADecodeDiagBilinearAccess(float4 e) {
    e.rb = e.rb * abs(5.0 * e.rb - 5.0 * 0.75);
    return round(e);
}

float2 SMAASearchDiag1(
    Texture2D edgesTex, SamplerState edgesSmp,
    float4 rt_metrics,
    float2 texcoord, float2 dir, out float2 e
) {
    float4 coord = float4(texcoord, -1.0, 1.0);
    float3 t = float3(rt_metrics.xy, 1.0);
    while (coord.z < float(SMAA_MAX_SEARCH_STEPS_DIAG - 1) &&
           coord.w > 0.9) {
        coord.xyz = mad(t, float3(dir, 1.0), coord.xyz);
        e = edgesTex.SampleLevel(edgesSmp, coord.xy, 0.0).rg;
        coord.w = dot(e, float2(0.5, 0.5));
    }
    return coord.zw;
}

float2 SMAASearchDiag2(
    Texture2D edgesTex, SamplerState edgesSmp,
    float4 rt_metrics,
    float2 texcoord, float2 dir, out float2 e
) {
    float4 coord = float4(texcoord, -1.0, 1.0);
    coord.x += 0.25 * rt_metrics.x; // See @SearchDiag2Optimization
    float3 t = float3(rt_metrics.xy, 1.0);
    while (coord.z < float(SMAA_MAX_SEARCH_STEPS_DIAG - 1) &&
           coord.w > 0.9) {
        coord.xyz = mad(t, float3(dir, 1.0), coord.xyz);

        // @SearchDiag2Optimization: fetch both edges at once via bilinear filtering.
        e = edgesTex.SampleLevel(edgesSmp, coord.xy, 0.0).rg;
        e = SMAADecodeDiagBilinearAccess(e);

        coord.w = dot(e, float2(0.5, 0.5));
    }
    return coord.zw;
}

float2 SMAAAreaDiag(Texture2D areaTex, SamplerState areaSmp, float2 dist, float2 e, float offset) {
    float2 texcoord = mad(float2(SMAA_AREATEX_MAX_DISTANCE_DIAG, SMAA_AREATEX_MAX_DISTANCE_DIAG), e, dist);

    texcoord = mad(SMAA_AREATEX_PIXEL_SIZE, texcoord, 0.5 * SMAA_AREATEX_PIXEL_SIZE);

    // Diagonal areas are on the second half of the texture.
    texcoord.x += 0.5;

    // Move to proper place, according to the subpixel offset.
    texcoord.y += SMAA_AREATEX_SUBTEX_SIZE * offset;

    return SMAA_AREATEX_SELECT(areaTex.SampleLevel(areaSmp, texcoord, 0.0));
}

float2 SMAACalculateDiagWeights(
    Texture2D edgesTex, SamplerState edgesSmp,
    Texture2D areaTex, SamplerState areaSmp,
    float4 rt_metrics,
    float2 texcoord, float2 e, float4 subsampleIndices
) {
    float2 weights = float2(0.0, 0.0);

    // Search for the line ends:
    float4 d;
    float2 end;
    if (e.r > 0.0) {
        d.xz = SMAASearchDiag1(edgesTex, edgesSmp, rt_metrics, texcoord, float2(-1.0, 1.0), end);
        d.x += float(end.y > 0.9);
    } else
        d.xz = float2(0.0, 0.0);
    d.yw = SMAASearchDiag1(edgesTex, edgesSmp, rt_metrics, texcoord, float2(1.0, -1.0), end);

    [branch]
    if (d.x + d.y > 2.0) { // d.x + d.y + 1 > 3
        // Fetch the crossing edges:
        float4 coords = mad(float4(-d.x + 0.25, d.x, d.y, -d.y - 0.25), rt_metrics.xyxy, texcoord.xyxy);
        float4 c;
        c.xy = edgesTex.SampleLevel(edgesSmp, coords.xy, 0.0, int2(-1, 0)).rg;
        c.zw = edgesTex.SampleLevel(edgesSmp, coords.zw, 0.0, int2(1, 0)).rg;
        c.yxwz = SMAADecodeDiagBilinearAccess(c.xyzw);

        // Merge crossing edges at each side into a single value.
        float2 cc = mad(float2(2.0, 2.0), c.xz, c.yw);

        // Remove the crossing edge if we didn't find the end of the line.
        SMAAMovc(bool2(step(0.9, d.zw)), cc, float2(0.0, 0.0));

        weights += SMAAAreaDiag(areaTex, areaSmp, d.xy, cc, subsampleIndices.z);
    }

    // Search for the line ends:
    d.xz = SMAASearchDiag2(edgesTex, edgesSmp, rt_metrics, texcoord, float2(-1.0, -1.0), end);
    if (edgesTex.SampleLevel(edgesSmp, texcoord, 0.0, int2(1, 0)).r > 0.0) {
        d.yw = SMAASearchDiag2(edgesTex, edgesSmp, rt_metrics, texcoord, float2(1.0, 1.0), end);
        d.y += float(end.y > 0.9);
    } else
        d.yw = float2(0.0, 0.0);

    [branch]
    if (d.x + d.y > 2.0) { // d.x + d.y + 1 > 3
        float4 coords = mad(float4(-d.x, -d.x, d.y, d.y), rt_metrics.xyxy, texcoord.xyxy);
        float4 c;
        c.x  = edgesTex.SampleLevel(edgesSmp, coords.xy, 0.0, int2(-1, 0)).g;
        c.y  = edgesTex.SampleLevel(edgesSmp, coords.xy, 0.0, int2(0, -1)).r;
        c.zw = edgesTex.SampleLevel(edgesSmp, coords.zw, 0.0, int2(1, 0)).gr;
        float2 cc = mad(float2(2.0, 2.0), c.xz, c.yw);

        SMAAMovc(bool2(step(0.9, d.zw)), cc, float2(0.0, 0.0));

        weights += SMAAAreaDiag(areaTex, areaSmp, d.xy, cc, subsampleIndices.w).gr;
    }

    return weights;
}

// ---- Horizontal/Vertical Search Functions (verbatim) --------------------------------------

float SMAASearchLength(Texture2D searchTex, SamplerState searchSmp, float2 e, float offset) {
    // The texture is flipped vertically, with left and right cases taking half of the space
    // horizontally.
    float2 scale = SMAA_SEARCHTEX_SIZE * float2(0.5, -1.0);
    float2 bias = SMAA_SEARCHTEX_SIZE * float2(offset, 1.0);

    scale += float2(-1.0, 1.0);
    bias  += float2(0.5, -0.5);

    scale *= 1.0 / SMAA_SEARCHTEX_PACKED_SIZE;
    bias *= 1.0 / SMAA_SEARCHTEX_PACKED_SIZE;

    return SMAA_SEARCHTEX_SELECT(searchTex.SampleLevel(searchSmp, mad(scale, e, bias), 0.0));
}

float SMAASearchXLeft(
    Texture2D edgesTex, SamplerState edgesSmp,
    Texture2D searchTex, SamplerState searchSmp,
    float4 rt_metrics,
    float2 texcoord, float end
) {
    // @PSEUDO_GATHER4: `texcoord` was already offset by (-0.25, -0.125) in
    // SMAABlendingWeightOffsets, sampling between edges to fetch four edges in a row.
    float2 e = float2(0.0, 1.0);
    while (texcoord.x > end &&
           e.g > 0.8281 &&
           e.r == 0.0) {
        e = edgesTex.SampleLevel(edgesSmp, texcoord, 0.0).rg;
        texcoord = mad(-float2(2.0, 0.0), rt_metrics.xy, texcoord);
    }

    float offset = mad(-(255.0 / 127.0), SMAASearchLength(searchTex, searchSmp, e, 0.0), 3.25);
    return mad(rt_metrics.x, offset, texcoord.x);
}

float SMAASearchXRight(
    Texture2D edgesTex, SamplerState edgesSmp,
    Texture2D searchTex, SamplerState searchSmp,
    float4 rt_metrics,
    float2 texcoord, float end
) {
    float2 e = float2(0.0, 1.0);
    while (texcoord.x < end &&
           e.g > 0.8281 &&
           e.r == 0.0) {
        e = edgesTex.SampleLevel(edgesSmp, texcoord, 0.0).rg;
        texcoord = mad(float2(2.0, 0.0), rt_metrics.xy, texcoord);
    }
    float offset = mad(-(255.0 / 127.0), SMAASearchLength(searchTex, searchSmp, e, 0.5), 3.25);
    return mad(-rt_metrics.x, offset, texcoord.x);
}

float SMAASearchYUp(
    Texture2D edgesTex, SamplerState edgesSmp,
    Texture2D searchTex, SamplerState searchSmp,
    float4 rt_metrics,
    float2 texcoord, float end
) {
    float2 e = float2(1.0, 0.0);
    while (texcoord.y > end &&
           e.r > 0.8281 &&
           e.g == 0.0) {
        e = edgesTex.SampleLevel(edgesSmp, texcoord, 0.0).rg;
        texcoord = mad(-float2(0.0, 2.0), rt_metrics.xy, texcoord);
    }
    float offset = mad(-(255.0 / 127.0), SMAASearchLength(searchTex, searchSmp, e.gr, 0.0), 3.25);
    return mad(rt_metrics.y, offset, texcoord.y);
}

float SMAASearchYDown(
    Texture2D edgesTex, SamplerState edgesSmp,
    Texture2D searchTex, SamplerState searchSmp,
    float4 rt_metrics,
    float2 texcoord, float end
) {
    float2 e = float2(1.0, 0.0);
    while (texcoord.y < end &&
           e.r > 0.8281 &&
           e.g == 0.0) {
        e = edgesTex.SampleLevel(edgesSmp, texcoord, 0.0).rg;
        texcoord = mad(float2(0.0, 2.0), rt_metrics.xy, texcoord);
    }
    float offset = mad(-(255.0 / 127.0), SMAASearchLength(searchTex, searchSmp, e.gr, 0.5), 3.25);
    return mad(-rt_metrics.y, offset, texcoord.y);
}

float2 SMAAArea(Texture2D areaTex, SamplerState areaSmp, float2 dist, float e1, float e2, float offset) {
    // Rounding prevents precision errors of bilinear filtering.
    float2 texcoord = mad(float2(SMAA_AREATEX_MAX_DISTANCE, SMAA_AREATEX_MAX_DISTANCE), round(4.0 * float2(e1, e2)), dist);

    texcoord = mad(SMAA_AREATEX_PIXEL_SIZE, texcoord, 0.5 * SMAA_AREATEX_PIXEL_SIZE);

    texcoord.y = mad(SMAA_AREATEX_SUBTEX_SIZE, offset, texcoord.y);

    return SMAA_AREATEX_SELECT(areaTex.SampleLevel(areaSmp, texcoord, 0.0));
}

// ---- Corner Detection Functions (verbatim; corner detection is UNCONDITIONALLY on) --------

void SMAADetectHorizontalCornerPattern(
    Texture2D edgesTex, SamplerState edgesSmp,
    inout float2 weights, float4 texcoord, float2 d
) {
    float2 leftRight = step(d.xy, d.yx);
    float2 rounding = (1.0 - SMAA_CORNER_ROUNDING_NORM) * leftRight;

    rounding /= leftRight.x + leftRight.y; // Reduce blending for pixels in the center of a line.

    float2 factor = float2(1.0, 1.0);
    factor.x -= rounding.x * edgesTex.SampleLevel(edgesSmp, texcoord.xy, 0.0, int2(0, 1)).r;
    factor.x -= rounding.y * edgesTex.SampleLevel(edgesSmp, texcoord.zw, 0.0, int2(1, 1)).r;
    factor.y -= rounding.x * edgesTex.SampleLevel(edgesSmp, texcoord.xy, 0.0, int2(0, -2)).r;
    factor.y -= rounding.y * edgesTex.SampleLevel(edgesSmp, texcoord.zw, 0.0, int2(1, -2)).r;

    weights *= saturate(factor);
}

void SMAADetectVerticalCornerPattern(
    Texture2D edgesTex, SamplerState edgesSmp,
    inout float2 weights, float4 texcoord, float2 d
) {
    float2 leftRight = step(d.xy, d.yx);
    float2 rounding = (1.0 - SMAA_CORNER_ROUNDING_NORM) * leftRight;

    rounding /= leftRight.x + leftRight.y;

    float2 factor = float2(1.0, 1.0);
    factor.x -= rounding.x * edgesTex.SampleLevel(edgesSmp, texcoord.xy, 0.0, int2(1, 0)).g;
    factor.x -= rounding.y * edgesTex.SampleLevel(edgesSmp, texcoord.zw, 0.0, int2(1, 1)).g;
    factor.y -= rounding.x * edgesTex.SampleLevel(edgesSmp, texcoord.xy, 0.0, int2(-2, 0)).g;
    factor.y -= rounding.y * edgesTex.SampleLevel(edgesSmp, texcoord.zw, 0.0, int2(-2, 1)).g;

    weights *= saturate(factor);
}

// ---- Blending Weight Calculation Pixel Shader (Second Pass, verbatim) ---------------------

float4 SMAABlendingWeightCalculationPS(
    float2 texcoord,
    float2 pixcoord,
    float4 offset[3],
    Texture2D edgesTex, SamplerState edgesSmp,
    Texture2D areaTex, SamplerState areaSmp,
    Texture2D searchTex, SamplerState searchSmp,
    float4 rt_metrics,
    float4 subsampleIndices // Just pass zero for SMAA 1x, see @SUBSAMPLE_INDICES.
) {
    float4 weights = float4(0.0, 0.0, 0.0, 0.0);

    float2 e = edgesTex.Sample(edgesSmp, texcoord).rg;

    [branch]
    if (e.g > 0.0) { // Edge at north
        // Diagonals have both north and west edges, so searching for them in one of the
        // boundaries is enough.
        weights.rg = SMAACalculateDiagWeights(
            edgesTex, edgesSmp, areaTex, areaSmp, rt_metrics, texcoord, e, subsampleIndices);

        // Priority to diagonals: if a diagonal is found, skip horizontal/vertical processing.
        [branch]
        if (weights.r == -weights.g) { // weights.r + weights.g == 0.0

            float2 d;

            // Find the distance to the left:
            float3 coords;
            coords.x = SMAASearchXLeft(
                edgesTex, edgesSmp, searchTex, searchSmp, rt_metrics, offset[0].xy, offset[2].x);
            coords.y = offset[1].y; // offset[1].y = texcoord.y - 0.25 * rt_metrics.y (@CROSSING_OFFSET)
            d.x = coords.x;

            float e1 = edgesTex.SampleLevel(edgesSmp, coords.xy, 0.0).r;

            // Find the distance to the right:
            coords.z = SMAASearchXRight(
                edgesTex, edgesSmp, searchTex, searchSmp, rt_metrics, offset[0].zw, offset[2].y);
            d.y = coords.z;

            d = abs(round(mad(rt_metrics.zz, d, -pixcoord.xx)));

            float2 sqrt_d = sqrt(d);

            float e2 = edgesTex.SampleLevel(edgesSmp, coords.zy, 0.0, int2(1, 0)).r;

            weights.rg = SMAAArea(areaTex, areaSmp, sqrt_d, e1, e2, subsampleIndices.y);

            // Fix corners:
            coords.y = texcoord.y;
            SMAADetectHorizontalCornerPattern(edgesTex, edgesSmp, weights.rg, coords.xyzy, d);

        } else
            e.r = 0.0; // Skip vertical processing.
    }

    [branch]
    if (e.r > 0.0) { // Edge at west
        float2 d;

        // Find the distance to the top:
        float3 coords;
        coords.y = SMAASearchYUp(
            edgesTex, edgesSmp, searchTex, searchSmp, rt_metrics, offset[1].xy, offset[2].z);
        coords.x = offset[0].x; // offset[1].x = texcoord.x - 0.25 * rt_metrics.x
        d.x = coords.y;

        float e1 = edgesTex.SampleLevel(edgesSmp, coords.xy, 0.0).g;

        // Find the distance to the bottom:
        coords.z = SMAASearchYDown(
            edgesTex, edgesSmp, searchTex, searchSmp, rt_metrics, offset[1].zw, offset[2].w);
        d.y = coords.z;

        d = abs(round(mad(rt_metrics.ww, d, -pixcoord.yy)));

        float2 sqrt_d = sqrt(d);

        float e2 = edgesTex.SampleLevel(edgesSmp, coords.xz, 0.0, int2(0, 1)).g;

        weights.ba = SMAAArea(areaTex, areaSmp, sqrt_d, e1, e2, subsampleIndices.x);

        // Fix corners:
        coords.x = texcoord.x;
        SMAADetectVerticalCornerPattern(edgesTex, edgesSmp, weights.ba, coords.xyxz, d);
    }

    return weights;
}

// ---- Neighborhood Blending Pixel Shader (Third Pass, verbatim; SMAA_REPROJECTION off) -----

float4 SMAANeighborhoodBlendingPS(
    float2 texcoord,
    float4 offset,
    Texture2D colorTex, SamplerState colorSmp,
    Texture2D blendTex, SamplerState blendSmp,
    float4 rt_metrics
) {
    // Fetch the blending weights for the current pixel.
    float4 a;
    a.x = blendTex.Sample(blendSmp, offset.xy).a; // Right
    a.y = blendTex.Sample(blendSmp, offset.zw).g; // Top
    a.wz = blendTex.Sample(blendSmp, texcoord).xz; // Bottom / Left

    [branch]
    if (dot(a, float4(1.0, 1.0, 1.0, 1.0)) < 1e-5) {
        return colorTex.SampleLevel(colorSmp, texcoord, 0.0);
    } else {
        bool h = max(a.x, a.z) > max(a.y, a.w); // max(horizontal) > max(vertical)

        float4 blendingOffset = float4(0.0, a.y, 0.0, a.w);
        float2 blendingWeight = a.yw;
        SMAAMovc(bool4(h, h, h, h), blendingOffset, float4(a.x, 0.0, a.z, 0.0));
        SMAAMovc(bool2(h, h), blendingWeight, a.xz);
        blendingWeight /= dot(blendingWeight, float2(1.0, 1.0));

        float4 blendingCoord = mad(blendingOffset, float4(rt_metrics.xy, -rt_metrics.xy), texcoord.xyxy);

        // Exploit bilinear filtering to mix the current pixel with the chosen neighbor.
        float4 color = blendingWeight.x * colorTex.SampleLevel(colorSmp, blendingCoord.xy, 0.0);
        color += blendingWeight.y * colorTex.SampleLevel(colorSmp, blendingCoord.zw, 0.0);

        return color;
    }
}

#endif // SMAA_COMMON_HLSLI
