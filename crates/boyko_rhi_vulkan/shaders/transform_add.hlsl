// Slice-0 step 0d compute shader: transform the buffer written by step 0c.
//
// Each invocation reads its element, adds 100, and writes it back:
//   buffer[i] = buffer[i] + 100
// Chained after `write_pattern` through a vkCmdPipelineBarrier (SHADER_WRITE ->
// SHADER_READ on the same buffer), so the final golden state is
//   buffer[i] = (i*2 + 1) + 100.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 transform_add.hlsl -Fo transform_add.comp.spv
//
// Binding 0 (set 0) = the SAME RWStructuredBuffer<uint> layout as
// write_pattern (a single STORAGE_BUFFER binding at COMPUTE), so both pipelines
// share one descriptor set layout and one descriptor set.

RWStructuredBuffer<uint> Data : register(u0);

struct PushConstants {
    uint count;
};
[[vk::push_constant]] PushConstants pc;

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint i = tid.x;
    if (i >= pc.count) {
        return;
    }
    Data[i] = Data[i] + 100u;
}
