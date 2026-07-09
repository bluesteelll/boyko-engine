// Phase-6 S0 rung-6 deferred-lighting fragment shader: the full-screen pass that
// reads the G-buffer (albedo + packed normal) and applies ONE hardcoded directional
// light, writing the lit color to the final output texture.
//
// This is the second half of minimal deferred shading: the geometry pass
// (`gbuffer.fs.hlsl`) wrote surface albedo + world normal into two G-buffer textures;
// this pass samples BOTH (two COMBINED_IMAGE_SAMPLER bindings at set 0, binding 0 +
// binding 1) at the full-screen UV and computes:
//
//   lit = albedo * max(dot(N, L), 0) + ambient
//
// for a fixed directional light direction L and a small additive ambient term. N is
// unpacked from the normal G-buffer `N = sampled.xyz * 2 - 1` (the geometry pass
// packed it `n * 0.5 + 0.5`).
//
// The vertex stage is the shared full-screen-triangle vertex shader
// (`fullscreen_sample.vs.spv`, reused): three SV_VertexID-generated positions + UVs
// cover the whole render target, so every output texel runs this fragment once.
//
// The basic slice picks L = (0, 0, 1) (aligned with the quad normal), so on the quad
// N·L = 1 exactly and `lit = albedo + ambient` — a value the host golden reproduces
// (within a small UNORM-quantization tolerance). At an UNCOVERED texel the G-buffer's
// albedo was cleared to 0 and its normal cleared to pack(0,0,0) = (0.5, 0.5, 0.5)
// (which unpacks to N = 0, so N·L = 0), so the lit corner is `0 + ambient = ambient`
// — the deterministic "background" the test asserts at the corner.
//
// Two `Texture2D` + `SamplerState` pairs declared at the SAME `vk::binding(i, 0)`
// collapse to one COMBINED_IMAGE_SAMPLER each under DXC's Vulkan SPIR-V backend,
// matching the Rust 2-binding bind-group layout.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 deferred_light.fs.hlsl -Fo deferred_light.fs.spv

[[vk::binding(0, 0)]] Texture2D    g_albedo     : register(t0);
[[vk::binding(0, 0)]] SamplerState g_albedo_smp : register(s0);
[[vk::binding(1, 0)]] Texture2D    g_normal     : register(t1);
[[vk::binding(1, 0)]] SamplerState g_normal_smp : register(s1);

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

// The hardcoded directional light direction (the direction TOWARD the light) and the
// additive ambient term. L aligns with the quad's +Z normal so N·L = 1 on the quad.
static const float3 LIGHT_DIR = float3(0.0, 0.0, 1.0);
static const float  AMBIENT   = 0.1;

float4 main(VsOut input) : SV_Target0 {
    float3 albedo = g_albedo.Sample(g_albedo_smp, input.uv).rgb;
    // Unpack the world normal: [0, 1] -> [-1, 1].
    float3 n = g_normal.Sample(g_normal_smp, input.uv).xyz * 2.0 - 1.0;

    float ndotl = max(dot(n, LIGHT_DIR), 0.0);
    float3 lit = albedo * ndotl + AMBIENT;
    return float4(lit, 1.0);
}
