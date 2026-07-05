// Rung 1a: the specialization-constant GPU-smoke shader (`spec_constant_smoke.comp.hlsl`).
//
// The MINIMAL proof that the RHI's new spec-constant path lowers a `SpecConstant`
// into a live `VkSpecializationInfo` the driver honors at pipeline-create. A single
// spec-const `SPEC_N` (constant_id 0) defaults to 3; a single thread writes its value
// into `buffer[0]`. The host builds the pipeline TWICE — once with an EMPTY spec slice
// (readback must be the shader default, 3) and once overriding constant_id 0 to 7
// (readback must be 7). The two readbacks are the oracle; no validation layer needed.
//
// Binding 0 (set 0) = one `RWStructuredBuffer<uint>` at COMPUTE — the SAME single
// STORAGE_BUFFER layout as `write_pattern` / `transform_add`, so the smoke rides the
// RHI's shared fixed compute layout (no vocabulary bind-group needed).
//
// Compiled offline (hermetic — the `.spv` is committed by the orchestrator) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       spec_constant_smoke.comp.hlsl -Fo spec_constant_smoke.comp.spv
// `[[vk::constant_id(0)]]` emits an `OpSpecConstant` the RHI's `VkSpecializationInfo`
// (constant_id 0 → 4 bytes) overrides at create.

// constant_id 0: the specialization constant under test. Defaults to 3 — the value
// the EMPTY-spec pipeline reads; the host overrides it to 7 in the second build.
[[vk::constant_id(0)]] const uint SPEC_N = 3;

// binding 0: the single-`uint` output the shader writes `SPEC_N` into.
RWStructuredBuffer<uint> Data : register(u0);

// A 4-byte COMPUTE push constant (an unused invocation count). Present only because the
// RHI's shared compute pipeline layout REQUIRES a non-empty (multiple-of-4) push range
// (`create_compute_pipeline` rejects `push_constant_bytes == 0`); the value is never read.
struct Push {
    uint count;
};
[[vk::push_constant]] Push pc;

[numthreads(1, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    // The `pc.count` bound keeps the push constant live in the SPIR-V (so the pipeline
    // layout's required push range is genuinely consumed). The single dispatched thread
    // (tid 0) writes the specialized value.
    if (tid.x < pc.count) {
        Data[0] = SPEC_N;
    }
}
