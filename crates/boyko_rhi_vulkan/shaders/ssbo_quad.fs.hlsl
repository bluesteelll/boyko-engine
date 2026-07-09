// GUI P5a RUNG 0.5 (de-risk) fragment shader: read the SAME per-instance record
// from the StructuredBuffer (set 0, binding 0) by the interpolated SV_InstanceID and
// output its solid color. Reading the SSBO in the FRAGMENT stage as well proves the
// VERTEX|FRAGMENT visibility bit lowers correctly (the descriptor is bound once,
// visible to both stages).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 ssbo_quad.fs.hlsl -Fo ssbo_quad.fs.spv

struct RungInstance {
    float2 min_px;
    float2 size_px;
    float4 color;
};

[[vk::binding(0, 0)]] StructuredBuffer<RungInstance> g_instances : register(t0);

struct VsOut {
    float4 position : SV_Position;
    nointerpolation uint inst_index : INSTANCE;
};

float4 main(VsOut input) : SV_Target0 {
    return g_instances[input.inst_index].color;     // SSBO read in the FRAGMENT stage
}
