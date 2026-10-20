// Phase-6 S0 rung-5 fragment shader: SAMPLE a bound texture + sampler at the
// interpolated UV and output the sampled color (a passthrough sample — no lighting
// math, per the rung-5 scope).
//
// The texture + sampler are a COMBINED_IMAGE_SAMPLER at (set 0, binding 0): a
// `Texture2D` and a `SamplerState` declared at the SAME `vk::binding` collapse to
// one combined descriptor under DXC's Vulkan SPIR-V backend, matching the Rust
// `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER` bind-group layout. The set is bound by
// `bind_descriptor_set` (GRAPHICS bind point) before the full-screen draw.
//
// `tex.Sample(smp, uv)` proves the full descriptor/sampler surface end-to-end: the
// sampled color round-trips through the descriptor set, so the rung-5 golden can
// assert that pass-2's output equals the known color pass-1 rendered into the
// source texture.
//
// Render wave PX1a (D-P1): the `*_SRGB` swapchain arm. On an sRGB swapchain the hardware encodes
// on write, so the display-referred image (`post_final`'s `display`, or `aa_out`) must be DECODED
// first or every pixel is encoded twice (about +40 LSB in the mid-tones). `SWAPCHAIN_SRGB=1`
// compiles the decode in: `srgb_eotf`, the eDSL's piecewise sRGB EOTF
// (`boyko_shaderdsl::post::srgb_eotf`, spliced by `emit_post --splice`; NOT `pow(c, 2.2)`, about
// 7 LSB off at 0.1), as a second `.spv`. The present pipeline picks the module by the swapchain
// format (`compute::present_sample_fs_spirv`). The default arm is textually the pre-PX1a body, so
// `fullscreen_sample.fs.spv` is byte-unchanged.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 fullscreen_sample.fs.hlsl -Fo fullscreen_sample.fs.spv
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 -D SWAPCHAIN_SRGB=1 fullscreen_sample.fs.hlsl \
//       -Fo fullscreen_sample_srgb.fs.spv

[[vk::binding(0, 0)]] Texture2D    g_tex : register(t0);
[[vk::binding(0, 0)]] SamplerState g_smp : register(s0);

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

#ifndef SWAPCHAIN_SRGB
#define SWAPCHAIN_SRGB 0
#endif

#if SWAPCHAIN_SRGB
// === GENERATED srgb_eotf BEGIN ===

float3 srgb_eotf(float3 c) {
    return float3(((c.x <= 0.04045) ? (c.x / 12.92) : (pow((c.x + 0.055) / 1.055, 2.4))), ((c.y <= 0.04045) ? (c.y / 12.92) : (pow((c.y + 0.055) / 1.055, 2.4))), ((c.z <= 0.04045) ? (c.z / 12.92) : (pow((c.z + 0.055) / 1.055, 2.4))));
}
// === GENERATED srgb_eotf END ===
#endif

float4 main(VsOut input) : SV_Target0 {
#if SWAPCHAIN_SRGB
    float4 display = g_tex.Sample(g_smp, input.uv);
    return float4(srgb_eotf(display.rgb), display.a);
#else
    return g_tex.Sample(g_smp, input.uv);
#endif
}
