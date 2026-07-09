// Phase-6 S0 rung-5 vertex shader: a full-screen triangle, NO vertex buffer.
//
// The three positions + UVs are generated from SV_VertexID (Vulkan's
// gl_VertexIndex), so a `draw(3, 1, 0, 0)` with NO bound vertex buffer covers the
// whole render target. This is the standard "oversized triangle" trick: a single
// triangle whose three vertices are placed so its interior covers the entire NDC
// `[-1, 1]^2` (the off-screen excess is clipped), giving full-screen coverage with
// one primitive and no vertex buffer.
//
// Clip-space positions (vid 0,1,2): (-1,-1), (3,-1), (-1,3).
// The matching UVs (computed as `(pos * 0.5 + 0.5)`) are: (0,0), (2,0), (0,2),
// which interpolate across the covered `[0,1]^2` region to a 1:1 texture sample.
// Vulkan's framebuffer Y points DOWN, so a clip-space `y` maps to UV `y` directly
// here (UV.y = pos.y * 0.5 + 0.5); the rung-5 source texture is written by the same
// pipeline convention (a top-centre triangle), so the round-trip is self-consistent
// — the test asserts the sampled CENTRE texel, which is convention-insensitive.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 fullscreen_sample.vs.hlsl -Fo fullscreen_sample.vs.spv

struct VsOut {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

VsOut main(uint vid : SV_VertexID) {
    VsOut output;
    // x = -1, 3, -1 ; y = -1, -1, 3  →  a triangle covering all of NDC [-1, 1]^2.
    float2 pos = float2((vid == 1) ? 3.0 : -1.0, (vid == 2) ? 3.0 : -1.0);
    output.position = float4(pos, 0.0, 1.0);
    output.uv = pos * 0.5 + 0.5;
    return output;
}
