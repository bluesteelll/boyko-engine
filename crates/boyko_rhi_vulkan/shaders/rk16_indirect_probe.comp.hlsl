// RK-16 (render-wave RD3): the indirect-dispatch probe (`rk16_indirect_probe.comp.hlsl`).
//
// Test-only and hand-written (the `spec_constant_smoke` precedent): one thread per workgroup
// writes `group + 1` into `Out[group]`, so a readback names exactly which workgroups ran. The
// host dispatches it through `RhiCommandEncoder::dispatch_indirect` with group counts read from a
// buffer (`tests/rk16_dispatch_indirect.rs`); zeros in the readback mean no group ran.
//
// Binding 0 (set 0) = one `RWStructuredBuffer<uint>` at COMPUTE: the RHI's fixed single
// STORAGE_BUFFER layout (the packed `bind_storage_buffer` path) and a one-entry vocabulary bind
// group (the `bind_descriptor_set_compute` path) both satisfy it. No push constant is read.
//
// Compiled offline (hermetic; `rk16_indirect_probe_spv_matches_source` re-runs it) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       rk16_indirect_probe.comp.hlsl -Fo rk16_indirect_probe.comp.spv

// The probe's output slots; a group past the last one writes nothing.
#define RK16_SLOTS 8

RWStructuredBuffer<uint> Out : register(u0);

[numthreads(1, 1, 1)]
void main(uint3 gid : SV_GroupID) {
    if (gid.x < RK16_SLOTS) {
        Out[gid.x] = gid.x + 1;
    }
}
