// SMAA 1x (AA campaign, Stage 2) — pass 3: neighborhood blending. Reads pass 2's `weights`
// (R8G8B8A8_UNORM) + the deferred resolve's LIT color and writes the final antialiased color
// to `aa_out`, which the present-blit then samples instead of `lit` directly.
//
// INTERFACE (must match the host SmaaActivation wiring):
//   set 0, binding 0 : LIT color, a COMBINED_IMAGE_SAMPLER.
//   set 0, binding 1 : weights,   a COMBINED_IMAGE_SAMPLER (pass 2's output).
//   Both share the SAME LINEAR/ClampToEdge sampler object host-side (Open Q2).
//   push constant    : SmaaPush { float4 rt_metrics; } = (1/w, 1/h, w, h) of present_extent.
// Vertex shader: reuse fullscreen_sample.vs.hlsl (shared by all three SMAA passes).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time), .spv hand-committed:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 smaa_blend.fs.hlsl -Fo smaa_blend.fs.spv

#include "smaa_common.hlsli"

[[vk::binding(0, 0)]] Texture2D    g_lit : register(t0);
[[vk::binding(0, 0)]] SamplerState g_lit_smp : register(s0);
[[vk::binding(1, 0)]] Texture2D    g_weights : register(t1);
[[vk::binding(1, 0)]] SamplerState g_weights_smp : register(s1);

[[vk::push_constant]] SmaaPush pc;

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

float4 main(VsOut input) : SV_Target0 {
    float4 offset;
    SMAANeighborhoodBlendingOffset(input.uv, pc.rt_metrics, offset);
    return SMAANeighborhoodBlendingPS(
        input.uv, offset, g_lit, g_lit_smp, g_weights, g_weights_smp, pc.rt_metrics);
}
