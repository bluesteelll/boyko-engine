// Anti-aliasing Stage 4 — TAA rung T3: the post-resolve CONTRAST-ADAPTIVE SHARPEN pass
// (`boyko_render::taa_config::SharpenMode::Rcas`).
//
// TAA's temporal reprojection + neighborhood clip is a mild low-pass — a converged history is a
// blurrier-than-native image. AMD FidelityFX CAS (Contrast-Adaptive Sharpening) restores edge
// acuity WITHOUT the ringing/overshoot a naive unsharp mask produces: the sharpening lobe is
// amplitude-limited per pixel by the local `min(min, 1-max)/max` headroom, so a high-contrast
// edge is sharpened only as far as it can go without clipping — the "robust" in RCAS.
//
// # Placement — the aa_out ping-pong (why a SEPARATE pass, not folded into the resolve)
//
// Sharpening is a 3x3 NEIGHBORHOOD read of the RESOLVED color, so it cannot run in the resolve's
// own dispatch (which produces one output pixel from history + the current sample; the neighbors'
// resolved values do not exist yet). So the resolve writes an INTERMEDIATE `taa_resolved` target
// (the ping) and this pass reads it and writes `aa_out` (the pong) — the present-blit's input,
// UNCHANGED. On `SharpenMode::None` (the default) this pass is NOT recorded at all: the resolve
// writes `aa_out` directly, exactly as pre-T3 (the structural 0%-gate — `taa_armed` byte-stable).
//
// # Bindings — a MINIMAL 2-descriptor set (no UBO)
//
// Unlike the resolve (which needs the camera/MotionCam UBOs to reconstruct motion), CAS is a pure
// image-space kernel: the extent and the owner-set sharpness ride a 12-byte push constant, so the
// set is just the two STORAGE images. Both are `rgba8` (LDR, post-tonemap) — CAS is defined on the
// displayed LDR color, which is exactly what the resolve wrote.
//
// binding 0 (READ): `taa_resolved` — the TAA resolve's intermediate LDR output (the resolve's
// `gAaOut` is re-pointed here when `Rcas` is armed). Bound as a STORAGE image kept in GENERAL (a
// `[]`-Load kernel, no hardware sampler — the SAME GENERAL-only discipline the resolve's history
// ring uses), read via edge-clamped integer taps.
[[vk::binding(0, 0)]] [[vk::image_format("rgba8")]] RWTexture2D<float4> gRcasIn : register(u0);
// binding 1 (WRITE): `aa_out` (R8G8B8A8_UNORM) — the present-blit's input. Written once per
// dispatched pixel (every pixel, unconditionally — the pre-pass UNDEFINED→GENERAL discard is
// always valid, mirroring the resolve's own `aa_out` barrier).
[[vk::binding(1, 0)]] [[vk::image_format("rgba8")]] RWTexture2D<float4> gAaOut : register(u1);

// The 12-byte COMPUTE push range: the runtime extent (byte-mirrors the camera UBO's
// `img_w_raw`/`img_h_raw` semantics — 0 => the legacy 64x64 fixture default) + the owner-set
// `TaaConfig::rcas_sharpness` mapped host-side into [0,1]. `_pad` keeps the range a round 16 bytes.
struct RcasPush {
    uint  img_w;      // runtime extent width  (0 => IMG_W_DEFAULT)
    uint  img_h;      // runtime extent height (0 => IMG_H_DEFAULT)
    float sharpness;  // [0,1]: 0 = mild (peak -1/8), 1 = strong (peak -1/5)
    uint  _pad;
};
[[vk::push_constant]] RcasPush pc;

// The legacy 64x64 fixture extent when the push extent is zero (mirrors the marcher / resolve /
// present's own `IMG_*_DEFAULT` fallbacks — a zeroed push renders the fixture, never a divide-by-
// or out-of-bounds).
static const uint IMG_W_DEFAULT = 64;
static const uint IMG_H_DEFAULT = 64;

// One edge-clamped tap of `gRcasIn` (CLAMP-to-edge: a border pixel reuses its nearest in-bounds
// neighbor, so the kernel never reads outside the image — the SAME border rule a `ClampToEdge`
// sampler would apply, done by hand since this is a `[]`-Load STORAGE read).
float3 tap(int2 p, int2 maxp) {
    int2 c = clamp(p, int2(0, 0), maxp);
    return gRcasIn[uint2(c)].rgb;
}

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint w = (pc.img_w == 0u) ? IMG_W_DEFAULT : pc.img_w;
    uint h = (pc.img_h == 0u) ? IMG_H_DEFAULT : pc.img_h;
    uint idx = tid.x;
    // The 1D pixel grid (the resolve/marcher dispatch shape — `dispatch_group_count_x` groups of
    // 64). Guard the tail: the last group over-covers `w*h`.
    if (idx >= w * h) {
        return;
    }
    int2 pcoord = int2(int(idx % w), int(idx / w));
    int2 maxp = int2(int(w) - 1, int(h) - 1);

    // The 3x3 neighborhood (AMD CAS):
    //   a b c
    //   d e f     (e = center)
    //   g h i
    float3 a = tap(pcoord + int2(-1, -1), maxp);
    float3 b = tap(pcoord + int2(0, -1), maxp);
    float3 c = tap(pcoord + int2(1, -1), maxp);
    float3 d = tap(pcoord + int2(-1, 0), maxp);
    float3 e = tap(pcoord, maxp);
    float3 f = tap(pcoord + int2(1, 0), maxp);
    float3 g = tap(pcoord + int2(-1, 1), maxp);
    float3 hh = tap(pcoord + int2(0, 1), maxp);
    float3 ii = tap(pcoord + int2(1, 1), maxp);

    // CAS local range: the 5-tap cross min/max, ADDED to the corner-extended min/max (each term
    // is thus ~2x the true min/max — the `2.0 -` and `rcp` below absorb the factor of two, exactly
    // as the FidelityFX CAS reference folds it).
    float3 mn = min(min(min(d, e), min(f, b)), hh);
    mn += min(mn, min(min(a, c), min(g, ii)));
    float3 mx = max(max(max(d, e), max(f, b)), hh);
    mx += max(mx, max(max(a, c), max(g, ii)));

    // Per-channel amplitude headroom in [0,1]: how far the center can be pushed before it clips
    // black or white. `sqrt` shapes it perceptually (the FidelityFX rsqrt-reciprocal, simplified).
    float3 rcp_mx = rcp(mx);
    float3 amp = saturate(min(mn, 2.0 - mx) * rcp_mx);
    amp = sqrt(amp);

    // The sharpening lobe weight: `sharpness` in [0,1] lerps the peak from -1/8 (mild) to -1/5
    // (strong) — the FidelityFX CAS sharpness mapping. The negative cross weight + the normalizing
    // reciprocal below form a unity-gain high-pass, amplitude-clamped by `amp`.
    float peak = -rcp(lerp(8.0, 5.0, saturate(pc.sharpness)));
    float3 wgt = amp * peak;
    float3 rcp_w = rcp(1.0 + 4.0 * wgt);
    float3 outc = saturate((b + d + f + hh) * wgt + e) * rcp_w;

    gAaOut[uint2(pcoord)] = float4(outc, 1.0);
}
