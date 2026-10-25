// ibl_level_probe.comp -- IBL1 RK-11 / RK-1 (ibl1/cut.md C6): the TEST-ONLY level-select kernel of
// `boyko_rhi_vulkan/tests/ibl_upload_mipped.rs`. Each of the 8 invocations samples the bound mipped
// image at the texture centre with an EXPLICIT `SampleLevel(k)`, `k` = the invocation index, so a
// chain uploaded level by level reads back one value per level. A sampler whose `maxLod` pins level
// 0 (`MipMode::None`) reads level 0 for every `k`: the silent-wrong class the gate exists to red.
// No render path binds it.
//
// # Layout (set 0)
//
//   binding 0: Texture2D<float4> + SamplerState, collapsed to one combined image sampler.
//   binding 1: RWStructuredBuffer<float4> gOut -- one texel per invocation (8 in all).
//
// # Compile (offline + hermetic; byte-gated by ibl_upload_mipped.rs)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       ibl_level_probe.comp.hlsl -Fo ibl_level_probe.comp.spv

[[vk::binding(0, 0)]] Texture2D<float4> gTex : register(t0);
[[vk::binding(0, 0)]] SamplerState gSmp : register(s0);
[[vk::binding(1, 0)]] RWStructuredBuffer<float4> gOut : register(u1);

[numthreads(8, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    gOut[id.x] = gTex.SampleLevel(gSmp, float2(0.5, 0.5), float(id.x));
}
