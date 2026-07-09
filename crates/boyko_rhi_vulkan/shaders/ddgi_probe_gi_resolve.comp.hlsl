// SDFDDGI I3 — the DDGI resolve-sample GPU GOLDEN shader (`probe_sample_gpu_eq_cpu_to_bits`).
//
// A STANDALONE compute harness that runs the SAME `ddgi_probe_sample` the deferred PBR resolve
// runs (both `#include "ddgi_resolve.hlsli"` — ONE source of truth), over a host-supplied set of
// receiver (position, normal) samples, and STOREs the resolved indirect irradiance so the host
// can diff it BIT-FOR-BIT against `boyko_rhi_vulkan::goldens::probe_sample`. This is where
// host↔GPU bit-exactness of the resolve sample is certified (`docs/RENDER-SDFDDGI-PLAN.md`, I3).
//
// The atlas is populated by the host with KNOWN per-tile uniform values (irradiance + depth
// moments), so the LINEAR `SampleLevel` in `ddgi_probe_sample` returns each tile's stored value
// EXACTLY (no interpolation error): the only arithmetic under test is the trilinear + wrap +
// Chebyshev blend, isolated for the bit-for-bit comparison.
//
// Bindings (its OWN pipeline layout — NOT the resolve set; no 19/19 pressure here):
//   b0 : cbuffer ResolvedDdgi (grid params: origin / inv_spacing / dims / mode)
//   t1+s1 : gDdgiIrr   (irradiance atlas, combined image)
//   t2+s2 : gDdgiDepth (depth-moment atlas, combined image)
//   t3 : StructuredBuffer<float4> receivers pos (xyz) — one per invocation
//   t4 : StructuredBuffer<float4> receivers normal (xyz)
//   u5 : RWStructuredBuffer<float4> out irradiance (xyz)
//
// The include reads gDdgiIrr / gDdgiIrrSamp / gDdgiDepth / gDdgiDepthSamp / gDdgiInvSpacDims /
// gDdgiOrigin — declared BEFORE the include (the tap contract). We alias the resolve's cbuffer
// names so the shared math is byte-identical.

// The grid UBO — byte-mirrors ResolvedDdgi (48 B). The trailing pad's `.x` carries the receiver
// SAMPLE COUNT (the invocation bound): a vocabulary-compute pipeline may NOT push_constants against
// the shared layout, so the count rides the UBO's otherwise-unused `_gDdgiPad.x`.
cbuffer ResolvedDdgi : register(b0) {
    float4 gDdgiOrigin;      // grid origin (probe (0,0,0) min world corner); .w padding
    float4 gDdgiInvSpacDims; // .x = inv_spacing; .yzw = bit-cast u32 dims (x, y, z)
    uint   gDdgiMode;        // mirrors ResolvedDdgi.ddgi_mode_word
    uint   gSampleCount;     // _gDdgiPad.x reused: the receiver sample count (invocation bound)
    uint2  _gDdgiPad;
};

Texture2DArray<float4> gDdgiIrr   : register(t1);
SamplerState           gDdgiIrrSamp : register(s1);
Texture2DArray<float2> gDdgiDepth  : register(t2);
SamplerState           gDdgiDepthSamp : register(s2);

StructuredBuffer<float4>   gRecvPos : register(t3);
StructuredBuffer<float4>   gRecvNrm : register(t4);
RWStructuredBuffer<float4> gOut     : register(u5);

#include "ddgi_resolve.hlsli"

[numthreads(64, 1, 1)]
void main(uint3 dtid : SV_DispatchThreadID) {
    uint i = dtid.x;
    if (i >= gSampleCount) {
        return;
    }
    float3 p = gRecvPos[i].xyz;
    float3 n = gRecvNrm[i].xyz;

    uint3 dims = uint3(asuint(gDdgiInvSpacDims.y),
                       asuint(gDdgiInvSpacDims.z),
                       asuint(gDdgiInvSpacDims.w));

    // The sky-ambient fallback the golden feeds MUST match what the host oracle is fed for the
    // no-coverage case — a fixed sentinel the host mirrors exactly.
    float3 sky = float3(0.05, 0.06, 0.08);

    float3 gi = ddgi_probe_sample(
        p, n,
        gDdgiOrigin.xyz, gDdgiInvSpacDims.x, dims,
        sky,
        gDdgiIrr, gDdgiIrrSamp,
        gDdgiDepth, gDdgiDepthSamp);

    gOut[i] = float4(gi, 1.0);
}
