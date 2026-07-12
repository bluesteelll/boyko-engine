// SMAA 1x (AA campaign, Stage 2) — pass 2: blending-weight calculation. Reads pass 1's `edges`
// (R8G8_UNORM) + the two boot-resident LUTs (`areaTex` 160x560 R8G8, `searchTex` 64x16 R8) and
// writes the per-pixel RGBA blending weights (R=left, G=top, B=right, A=bottom) to `weights`
// (R8G8B8A8_UNORM), consumed by pass 3 (`smaa_blend.fs.hlsl`). PRESET_HIGH: diagonal +
// corner detection both ON (`SMAACalculateDiagWeights` / `SMAADetect*CornerPattern`).
//
// INTERFACE (must match the host SmaaActivation wiring):
//   set 0, binding 0 : edges,     a COMBINED_IMAGE_SAMPLER (pass 1's output).
//   set 0, binding 1 : areaTex,   a COMBINED_IMAGE_SAMPLER (boot-resident LUT).
//   set 0, binding 2 : searchTex, a COMBINED_IMAGE_SAMPLER (boot-resident LUT).
//   All three share the SAME LINEAR/ClampToEdge sampler object host-side (Open Q2); each has
//   its own descriptor-bound `SamplerState` here (the CombinedImageSampler binding shape).
//   push constant    : SmaaPush { float4 rt_metrics; } = (1/w, 1/h, w, h) of present_extent.
// Vertex shader: reuse fullscreen_sample.vs.hlsl (shared by all three SMAA passes).
//
// `subsampleIndices` is `float4(0,0,0,0)` — SMAA 1x has no temporal supersampling.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time), .spv hand-committed:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 smaa_weight.fs.hlsl -Fo smaa_weight.fs.spv

#include "smaa_common.hlsli"

[[vk::binding(0, 0)]] Texture2D    g_edges : register(t0);
[[vk::binding(0, 0)]] SamplerState g_edges_smp : register(s0);
[[vk::binding(1, 0)]] Texture2D    g_area : register(t1);
[[vk::binding(1, 0)]] SamplerState g_area_smp : register(s1);
[[vk::binding(2, 0)]] Texture2D    g_search : register(t2);
[[vk::binding(2, 0)]] SamplerState g_search_smp : register(s2);

[[vk::push_constant]] SmaaPush pc;

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

float4 main(VsOut input) : SV_Target0 {
    float2 pixcoord;
    float4 offset[3];
    SMAABlendingWeightOffsets(input.uv, pc.rt_metrics, pixcoord, offset);

    float4 subsampleIndices = float4(0.0, 0.0, 0.0, 0.0);
    return SMAABlendingWeightCalculationPS(
        input.uv,
        pixcoord,
        offset,
        g_edges, g_edges_smp,
        g_area, g_area_smp,
        g_search, g_search_smp,
        pc.rt_metrics,
        subsampleIndices);
}
