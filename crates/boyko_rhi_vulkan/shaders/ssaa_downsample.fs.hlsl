// Anti-Aliasing campaign — SSAA (supersampling) 2x downsample fragment shader.
//
// SSAA renders the ENTIRE deferred pipeline at 2x per axis (composite_extent = 2*native,
// boot-fixed by the host) into the LIT ring, then this pass box-downsamples each 2x2
// super-block into ONE native-resolution `aa_out` texel. The present-blit then samples the
// native `aa_out` 1:1 (unchanged). OFF (scale 1) never records this pass — byte-identical.
//
// GAMMA-CORRECTNESS (the crux): LIT is the post-tonemap, manually gamma-2.2-OETF-encoded,
// DISPLAY-space value stored in an R8G8B8A8_UNORM target (NOT _SRGB) — see deferred_pbr.hlsl
// `OETF_GAMMA_EXP = 1.0/2.2` (`lit = pow(lit, OETF_GAMMA_EXP)`), so NO hardware linearization
// exists anywhere. Averaging the gamma-encoded bytes directly (the naive / "free single
// bilinear tap" box) is physically wrong and darkens edges (~31% too dark at a full-contrast
// edge: a true 50%-linear-coverage edge should encode to ~0.730, naive gamma-space averaging
// yields 0.5). This shader therefore averages in LINEAR light — decode (pow 2.2), average the
// 2x2, re-encode (pow 1/2.2) — the exact inverse/forward of the resolve's OETF.
//
// Ceiling: the taps are already per-sample tonemapped, so this is avg(tonemap(x)), not
// tonemap(avg(x)); accepted for v1 (LIT is the only buffer). The box filter is the cheapest
// correct kernel; a Wronski-8-tap / DSR-Gaussian is a shader-only swap (same interface).
//
// INTERFACE: set 0, binding 0 = LIT combined-image-sampler (the 2x ring slot). The shader uses
// `.Load` (texelFetch), which bypasses filtering, so the bound sampler (the shared NEAREST
// present sampler) is irrelevant — it exists only to satisfy the 1-CIS `present_layout` the
// boot pipeline reuses. No push constants (the 2x ratio is compiled in). Output: R8G8B8A8_UNORM
// (aa_out's format, native-sized). All `.Load` coords are in [0, 2*native) — in-bounds of the
// 2x LIT by the host boot-arm invariant (composite == 2*native whenever this shader runs).
//
// Compiled offline (hermetic; .spv hand-committed):
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 ssaa_downsample.fs.hlsl -Fo ssaa_downsample.fs.spv

[[vk::binding(0, 0)]] Texture2D    g_lit : register(t0);
[[vk::binding(0, 0)]] SamplerState g_smp : register(s0);

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

// The manual OETF exponent — the exact inverse/forward of deferred_pbr.hlsl's
// `OETF_GAMMA_EXP = 1.0/2.2`.
static const float DECODE_GAMMA = 2.2;
static const float ENCODE_GAMMA = 1.0 / 2.2;

// SSAA_SCALE == 2: compiled in (the host arms this pass only when composite == 2*native).
static const int SSAA_SCALE = 2;

float4 main(VsOut input) : SV_Target0 {
    // `input.position.xy` is the NATIVE output pixel index p in [0, native). The 2x2
    // super-block's top-left texel in the 2x LIT is p * SSAA_SCALE.
    int2 p = int2(input.position.xy);
    int2 s = p * SSAA_SCALE;

    // Decode each tap to linear light, average the 2x2 block, re-encode.
    float3 c =
        ( pow(g_lit.Load(int3(s,               0)).rgb, DECODE_GAMMA)
        + pow(g_lit.Load(int3(s + int2(1, 0),  0)).rgb, DECODE_GAMMA)
        + pow(g_lit.Load(int3(s + int2(0, 1),  0)).rgb, DECODE_GAMMA)
        + pow(g_lit.Load(int3(s + int2(1, 1),  0)).rgb, DECODE_GAMMA) ) * 0.25;

    // Re-encode to the display-space OETF the present-blit expects; alpha forced 1.0
    // (the present-blit ignores it).
    return float4(pow(c, ENCODE_GAMMA), 1.0);
}
