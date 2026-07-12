// SMAA 1x (AA campaign, Stage 2) — pass 1: luma edge detection. Reads the deferred resolve's
// LIT color (post-tonemap, non-sRGB, matching the reference's "IMPORTANT NOTICE" requirement
// for the color/luma edge-detection input) and writes the RG edge mask (R = west/left edge,
// G = north/top edge) to `edges` (R8G8_UNORM), consumed by pass 2 (`smaa_weight.fs.hlsl`).
//
// INTERFACE (must match the host SmaaActivation wiring):
//   set 0, binding 0 : LIT color, a COMBINED_IMAGE_SAMPLER — the shared LINEAR/ClampToEdge
//                      SMAA sampler (Open Q2 — the same sampler binds every SMAA tap).
//   push constant    : SmaaPush { float4 rt_metrics; } = (1/w, 1/h, w, h) of present_extent.
// Vertex shader: reuse fullscreen_sample.vs.hlsl (the SV_VertexID fullscreen triangle, shared
// by all three SMAA passes — no per-pass VS, unlike the reference's per-pass VS/PS split).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time), .spv hand-committed:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 smaa_edge.fs.hlsl -Fo smaa_edge.fs.spv

#include "smaa_common.hlsli"

[[vk::binding(0, 0)]] Texture2D    g_lit : register(t0);
[[vk::binding(0, 0)]] SamplerState g_smp : register(s0);

[[vk::push_constant]] SmaaPush pc;

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

float2 main(VsOut input) : SV_Target0 {
    float4 offset[3];
    SMAAEdgeDetectionOffsets(input.uv, pc.rt_metrics, offset);
    return SMAALumaEdgeDetectionPS(input.uv, offset, g_lit, g_smp);
}
