// HW-RT rung R2a-4a: the AS-descriptor GPU-smoke shader (`hwrt_as_descriptor_smoke.comp.hlsl`).
//
// A MINIMAL rayQuery compute whose ONLY purpose is to exercise the new
// `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR` descriptor write end-to-end on real hardware:
// the host builds a TLAS, writes its handle into binding t0 via the R2a-4a
// `VkWriteDescriptorSetAccelerationStructureKHR` `p_next` path, and dispatches this shader. If the
// descriptor write is malformed (wrong sType, dangling `p_acceleration_structures`, bad layout)
// the trace mis-reads the AS → a device-lost / a validation error — the silent-FFI class the smoke
// exists to catch (invisible to `abi_guard` + a no-validation box otherwise).
//
// The shader TRACES one inline ray against the bound TLAS (so the descriptor is genuinely
// consumed, not merely declared) and stores whether the ray committed a triangle hit into the
// single-`uint` output buffer (binding u1). The result value is irrelevant to the smoke — the
// oracle is "no device-lost + clean validation" — but tracing forces the driver to dereference the
// AS descriptor, which is exactly what must be validated.
//
// Compiled offline (hermetic — the `.spv` is committed) with:
//   dxc.exe -T cs_6_5 -E main -spirv -fspv-target-env=vulkan1.3 \
//       hwrt_as_descriptor_smoke.comp.hlsl -Fo hwrt_as_descriptor_smoke.comp.spv
// SM6.5 + vulkan1.3 emits `OpCapability RayQueryKHR` + `OpExtension "SPV_KHR_ray_query"`.

// binding t0: the TLAS the R2a-4a descriptor write binds — a
// `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR` descriptor.
[[vk::binding(0)]] RaytracingAccelerationStructure tlas : register(t0);

// binding u1: a single-`uint` output the shader writes the hit result into (so the dispatch has an
// observable side effect and the TLAS read cannot be dead-code-eliminated).
[[vk::binding(1)]] RWStructuredBuffer<uint> result : register(u1);

// A 4-byte COMPUTE push constant (an unused invocation count). Present only because the RHI's
// shared compute pipeline layout REQUIRES a non-empty (multiple-of-4) push range
// (`create_compute_pipeline` rejects `push_constant_bytes == 0`); the value is never read.
struct Push {
    uint count;
};
[[vk::push_constant]] Push pc;

[numthreads(1, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    // A trivial ray straight down +Z from the origin. The smoke's TLAS may or may not be hit; the
    // value is not asserted — only that the trace against the bound descriptor runs clean.
    RayDesc ray;
    ray.Origin = float3(0.0, 0.0, 0.0);
    ray.Direction = float3(0.0, 0.0, 1.0);
    ray.TMin = 1e-3;
    ray.TMax = 1e4;

    RayQuery<RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> q;
    q.TraceRayInline(tlas, 0, 0xFF, ray);
    q.Proceed();
    uint hit = (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT) ? 1u : 0u;

    // The `pc.count` bound keeps the push constant live in the SPIR-V (so the pipeline layout's
    // required push range is genuinely consumed). The single dispatched thread (tid 0) writes.
    if (tid.x < pc.count) {
        result[0] = hit;
    }
}
