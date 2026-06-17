// Phase-6 S0 rung-6 G-buffer (geometry) fragment shader: write the deferred
// G-buffer's MULTIPLE render targets (MRT) — albedo to SV_Target0 and the (packed)
// world normal to SV_Target1.
//
// This is the deferred-shading geometry pass: instead of lighting the surface here
// (forward shading), it writes the surface PROPERTIES into the G-buffer, and a later
// full-screen lighting pass reads them back to shade (see `deferred_light.fs.hlsl`).
// Two color attachments are bound at draw time; the pipeline declares two
// `color_formats` (W2-b) and one color-blend state per target.
//
//   SV_Target0 (albedo, R8G8B8A8_UNORM): a known constant albedo for the basic slice.
//   SV_Target1 (normal, R8G8B8A8_UNORM): the world normal packed `n * 0.5 + 0.5` into
//     [0, 1] so it survives a UNORM attachment (a normal component is in [-1, 1]).
//     The lighting pass unpacks it `n = sampled * 2 - 1`.
//
// The basic slice's quad faces +Z with normal (0, 0, 1), which packs to (0.5, 0.5,
// 1.0) — chosen so the decisive component (z = 1.0) is an EXACT UNORM value, keeping
// the downstream lit golden exact.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer.fs.hlsl -Fo gbuffer.fs.spv

struct PsIn {
    float4 position : SV_Position;
    float3 normal   : NORMAL;
};

struct PsOut {
    float4 albedo : SV_Target0;  // G-buffer attachment 0
    float4 normal : SV_Target1;  // G-buffer attachment 1
};

// A known opaque albedo for the basic slice's quad. The lighting golden is computed
// from this exact value on the host side.
static const float3 ALBEDO = float3(0.8, 0.6, 0.4);

PsOut main(PsIn input) {
    PsOut output;
    // Re-normalize the interpolated normal (rasterizer interpolation can shorten it),
    // then pack [-1, 1] -> [0, 1] for the UNORM attachment.
    float3 n = normalize(input.normal);
    output.albedo = float4(ALBEDO, 1.0);
    output.normal = float4(n * 0.5 + 0.5, 1.0);
    return output;
}
