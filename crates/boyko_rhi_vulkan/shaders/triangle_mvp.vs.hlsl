// Phase-6 S0 rung-3 vertex shader: a triangle read from a REAL vertex buffer and
// positioned by an MVP push constant.
//
// Unlike the rung-2 SV_VertexID variant, the three positions + per-vertex colors
// come from a bound vertex buffer (binding 0): position at offset 0
// (R32G32B32_SFLOAT) + color at offset 12 (R32G32B32A32_SFLOAT), 28-byte stride.
// The vertex position is transformed by a 4x4 MVP matrix supplied as a VERTEX-stage
// push constant (64 bytes), so the rung-3 acceptance test can choose model-space
// vertex coordinates + a known MVP that together cover the image centre — proving
// the MVP transform is genuinely applied, not bypassed.
//
// HLSL matrices are row-major by default; `mul(mvp, p)` treats `mvp` as the matrix
// on the left. The host packs the matrix so that `mul(mvp, float4(pos, 1))` yields
// the intended clip-space position (see the rung-3 test's MVP comment for the exact
// packing convention).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 triangle_mvp.vs.hlsl -Fo triangle_mvp.vs.spv

struct PushConstants {
    float4x4 mvp;
};
[[vk::push_constant]] PushConstants pc;

struct VsIn {
    float3 position : POSITION;  // vertex-buffer offset 0,  R32G32B32_SFLOAT
    float4 color    : COLOR0;    // vertex-buffer offset 12, R32G32B32A32_SFLOAT
};

struct VsOut {
    float4 position : SV_Position;
    float4 color    : COLOR0;
};

VsOut main(VsIn input) {
    VsOut output;
    output.position = mul(pc.mvp, float4(input.position, 1.0));
    output.color = input.color;
    return output;
}
