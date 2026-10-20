// Anti-Aliasing campaign — SSAA (supersampling) 2x downsample fragment shader.
//
// SSAA renders the ENTIRE scene at 2x per axis (composite_extent = 2*native, boot-fixed by the
// host) into the LIT ring, then this pass box-downsamples each 2x2 super-block into ONE
// native-resolution `lit_native` texel (render wave PX1a: a declared graph pass writing
// `lit_native` as its colour attachment, `LIT_FORMAT`); `post_final` then tonemaps `lit_native`
// into `display`. OFF (scale 1) never records this pass.
//
// RADIANCE-CORRECT BY CONSTRUCTION (render wave PX1a): `lit` holds pre-exposed SCENE-LINEAR
// radiance (RGBA16F), so the plain box average IS the linear-light average — no decode / re-encode
// pair (the display-referred `lit` needed `pow 2.2` / `pow 1/2.2` around the average), and the
// average is taken BEFORE the tonemap: tonemap(avg(x)), the correct order the display-referred
// store could not give. Every tap passes through `hdr_sanitize` (one non-finite tap would poison
// the native texel; it reads as black instead), so the box is `0.25 * sum(hdr_sanitize(tap))`.
// The box filter is the cheapest correct kernel; a Wronski-8-tap / DSR-Gaussian is a shader-only
// swap (same interface).
//
// INTERFACE: set 0, binding 0 = LIT combined-image-sampler (the 2x ring slot). The shader uses
// `.Load` (texelFetch), which bypasses filtering, so the bound sampler (the shared NEAREST
// present sampler) is irrelevant — it exists only to satisfy the 1-CIS `present_layout` the
// boot pipeline reuses. No push constants (the 2x ratio is compiled in). Output: `lit_native`
// (`LIT_FORMAT`, native-sized). All `.Load` coords are in [0, 2*native) — in-bounds of the 2x LIT
// by the host boot-arm invariant (composite == 2*native whenever this shader runs).
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

// SSAA_SCALE == 2: compiled in (the host arms this pass only when composite == 2*native).
static const int SSAA_SCALE = 2;

// The HDR boundary (`px1a/cut.md` D-S2): GENERATED from `boyko_shaderdsl::post` by `cargo run -p
// boyko_shaderdsl --features emit --bin emit_post -- --splice`; a hand-edit fails
// `tests/post_chain_spv_sync.rs`. Only `hdr_sanitize` is called here (DXC strips the guard).
// === GENERATED post_hdr_boundary BEGIN ===
static const uint F32_EXP_MASK = 0x7F800000u;
static const float HDR_MAX = 64512.0;

float3 hdr_sanitize(float3 c) {
    return float3(((min(min((asuint(c.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(c.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(c.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(c.x, 0.0), HDR_MAX))), ((min(min((asuint(c.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(c.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(c.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(c.y, 0.0), HDR_MAX))), ((min(min((asuint(c.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(c.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(c.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(c.z, 0.0), HDR_MAX))));
}

float3 hdr_history_guard(float3 hist, float3 cur) {
    return float3(((min(min((asuint(hist.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(hist.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(hist.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (float3(((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.x, 0.0), HDR_MAX))), ((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.y, 0.0), HDR_MAX))), ((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.z, 0.0), HDR_MAX)))).x) : (max(hist.x, 0.0))), ((min(min((asuint(hist.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(hist.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(hist.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (float3(((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.x, 0.0), HDR_MAX))), ((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.y, 0.0), HDR_MAX))), ((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.z, 0.0), HDR_MAX)))).y) : (max(hist.y, 0.0))), ((min(min((asuint(hist.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(hist.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(hist.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (float3(((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.x, 0.0), HDR_MAX))), ((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.y, 0.0), HDR_MAX))), ((min(min((asuint(cur.x) & F32_EXP_MASK) ^ F32_EXP_MASK, (asuint(cur.y) & F32_EXP_MASK) ^ F32_EXP_MASK), (asuint(cur.z) & F32_EXP_MASK) ^ F32_EXP_MASK) == 0u) ? (0.0) : (min(max(cur.z, 0.0), HDR_MAX)))).z) : (max(hist.z, 0.0))));
}
// === GENERATED post_hdr_boundary END ===

float4 main(VsOut input) : SV_Target0 {
    // `input.position.xy` is the NATIVE output pixel index p in [0, native). The 2x2
    // super-block's top-left texel in the 2x LIT is p * SSAA_SCALE.
    int2 p = int2(input.position.xy);
    int2 s = p * SSAA_SCALE;

    float3 c =
        ( hdr_sanitize(g_lit.Load(int3(s,               0)).rgb)
        + hdr_sanitize(g_lit.Load(int3(s + int2(1, 0),  0)).rgb)
        + hdr_sanitize(g_lit.Load(int3(s + int2(0, 1),  0)).rgb)
        + hdr_sanitize(g_lit.Load(int3(s + int2(1, 1),  0)).rgb) ) * 0.25;

    // Alpha forced 1.0 (`post_final` stores `(rgb, 1)`; nothing reads `lit_native`'s alpha).
    return float4(c, 1.0);
}
