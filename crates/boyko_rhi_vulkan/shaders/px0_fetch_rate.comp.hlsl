// px0_fetch_rate.comp -- render wave PX0 (H3 Scope 2; px0/cut.md D-P5): the TEST-ONLY gather
// kernel of the fetch-rate probe (K15: the 64-bit filter rate is unmeasured). Dispatched only by
// `boyko_rhi_vulkan/tests/fetch_rate_probe.rs` (gpu-ignored, a WR-1 row: timings are read in the
// orchestrator's quiet window, never gated). Each invocation performs `iters` fetches of the bound
// image -- filtered (`SampleLevel`, linear) or unfiltered (`Load`) -- along a scrambled walk, and
// writes their sum so no fetch is dead.
//
// # Layout (set 0)
//
//   binding 0: Texture2D<float4> + SamplerState, collapsed to one combined image sampler.
//   binding 1: RWStructuredBuffer<float> gSink -- one word per invocation.
//   push: { size, iters, filtered, _pad }.
//
// # Compile (offline + hermetic; byte-gated by px0_probe_spv_sync.rs)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       px0_fetch_rate.comp.hlsl -Fo px0_fetch_rate.comp.spv

[[vk::binding(0, 0)]] Texture2D<float4> gTex : register(t0);
[[vk::binding(0, 0)]] SamplerState gSmp : register(s0);
[[vk::binding(1, 0)]] RWStructuredBuffer<float> gSink : register(u1);

struct Px0FetchPush {
    uint size;
    uint iters;
    uint filtered;
    uint pad;
};
[[vk::push_constant]] Px0FetchPush pc;

[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint x = id.x % pc.size;
    uint y = (id.x / pc.size) % pc.size;
    float acc = 0.0;
    for (uint k = 0u; k < pc.iters; ++k) {
        uint xx = (x + k * 7u) % pc.size;
        uint yy = (y + k * 13u) % pc.size;
        float4 t;
        if (pc.filtered != 0u) {
            t = gTex.SampleLevel(gSmp, (float2(xx, yy) + 0.25) / float(pc.size), 0.0);
        } else {
            t = gTex.Load(int3(int(xx), int(yy), 0));
        }
        acc += t.x + t.y;
    }
    gSink[id.x] = acc;
}
