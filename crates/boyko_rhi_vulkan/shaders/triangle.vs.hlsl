// Phase-6 S0 rung-2 vertex shader: a single hardcoded triangle, NO vertex buffer.
//
// The three positions are generated from SV_VertexID (Vulkan's gl_VertexIndex),
// so a `draw(3, 1, 0, 0)` with NO bound vertex buffer suffices — the rung-2
// deviation from the plan's "vertex buffer" suggestion (the plan explicitly
// permits the gl_VertexIndex-generated variant as the simpler rung-2 path).
//
// NDC layout (Vulkan: +Y is DOWN in framebuffer space, but for a symmetric
// triangle the sign is immaterial): the triangle is centred on the origin and
// spans roughly the middle 70% of the surface, so it COVERS the centre texel and
// does NOT cover the four corner texels. The rung-2 golden asserts exactly that:
// centre == fragment colour, corner == clear colour (proving real rasterisation,
// not a bare clear).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 triangle.vs.hlsl -Fo triangle.vs.spv

static const float2 POSITIONS[3] = {
    float2( 0.0, -0.7),  // top-centre
    float2( 0.7,  0.7),  // bottom-right
    float2(-0.7,  0.7),  // bottom-left
};

float4 main(uint vid : SV_VertexID) : SV_Position {
    return float4(POSITIONS[vid], 0.0, 1.0);
}
