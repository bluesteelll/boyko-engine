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
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 fullscreen_sample.fs.hlsl -Fo fullscreen_sample.fs.spv

[[vk::binding(0, 0)]] Texture2D    g_tex : register(t0);
[[vk::binding(0, 0)]] SamplerState g_smp : register(s0);

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

float4 main(VsOut input) : SV_Target0 {
    return g_tex.Sample(g_smp, input.uv);
}
