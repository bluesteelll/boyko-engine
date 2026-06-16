// Slice-0 step 0c compute shader: write a known pattern into a storage buffer.
//
// Each invocation writes `buffer[i] = i*2 + 1` for its global index `i`. The
// CPU dispatches ceil(N / 64) groups of 64 and bounds-checks against N so a
// non-multiple-of-64 N never writes out of range (N is passed as a push
// constant so one .spv serves any element count).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 write_pattern.hlsl -Fo write_pattern.comp.spv
//
// Binding 0 (set 0) = a RWStructuredBuffer<uint> matching the descriptor set
// layout built in `compute.rs` (a single STORAGE_BUFFER binding at COMPUTE).

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
    Data[i] = i * 2u + 1u;
}
