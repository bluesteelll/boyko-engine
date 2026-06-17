// Phase-6 S0 rung-6 G-buffer (geometry) vertex shader: a quad read from a REAL
// vertex buffer (position + world normal) and positioned by an MVP push constant.
//
// The deferred-shading geometry pass rasterizes meshes into the G-buffer's MULTIPLE
// render targets (MRT): albedo to SV_Target0, world normal to SV_Target1 (see the
// matching `gbuffer.fs.hlsl`). This vertex stage carries the per-vertex POSITION
// (offset 0, R32G32B32_SFLOAT) + NORMAL (offset 12, R32G32B32_SFLOAT) from a bound
// vertex buffer (binding 0, 24-byte stride), transforms the position by the MVP, and
// passes the normal through to the fragment stage. The basic slice uses a known
// quad facing +Z (all-vertex normal (0, 0, 1)), so the rasterized normal is exact.
//
// HLSL matrices are row-major by default; `mul(mvp, p)` treats `mvp` as the matrix
// on the left, matching the rung-3 packing convention. The basic slice uses a
// diagonal MVP (symmetric, so row/column-major storage is identical) that scales the
// model quad to cover the image centre.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer.vs.hlsl -Fo gbuffer.vs.spv

struct PushConstants {
    float4x4 mvp;
};
[[vk::push_constant]] PushConstants pc;

struct VsIn {
    float3 position : POSITION;  // vertex-buffer offset 0,  R32G32B32_SFLOAT
    float3 normal   : NORMAL;    // vertex-buffer offset 12, R32G32B32_SFLOAT
};

struct VsOut {
    float4 position : SV_Position;
    float3 normal   : NORMAL;
};

VsOut main(VsIn input) {
    VsOut output;
    output.position = mul(pc.mvp, float4(input.position, 1.0));
    output.normal = input.normal;
    return output;
}
