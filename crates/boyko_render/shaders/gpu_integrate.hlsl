// Phase 5 demo compute shader: the GpuSystem's per-frame, per-element integrate.
//
// Each invocation reads its element and adds 100, writing it back IN PLACE on
// the GPU-resident ECS column (reusing the Slice-0 step-0d arithmetic so the CPU
// golden after N frames is `initial + 100*N` — the `golden_chained` shape the
// zero-readback test asserts):
//   Data[i] = Data[i] + 100
//
// The column lives in DeviceLocal VRAM and is mutated entirely on the GPU, so
// there is NO per-frame readback — the whole point of the GpuColumn capstone.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gpu_integrate.hlsl -Fo gpu_integrate.comp.spv
//
// Binding 0 (set 0) = a single RWStructuredBuffer<uint> at COMPUTE — the same
// descriptor-set layout as the Slice-0 compute shaders (write_pattern /
// transform_add), so the device column buffer is bound directly with no per-frame
// CPU touch. `count` is a push constant so one .spv serves any element count;
// the CPU dispatches ceil(count / 64) groups of 64 and the bounds check keeps a
// non-multiple-of-64 count from writing out of range.

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
