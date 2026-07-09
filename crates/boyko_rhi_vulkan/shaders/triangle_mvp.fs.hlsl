// Phase-6 S0 rung-3 fragment shader: output the interpolated per-vertex color.
//
// The rung-3 vertex buffer carries a per-vertex color; this stage just passes the
// rasterizer-interpolated color through. The rung-3 test authors a single solid
// vertex color across all three vertices, so the interpolated result at the centre
// texel is exactly that color (the byte value the golden checks), distinct from the
// clear color at an uncovered corner.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 triangle_mvp.fs.hlsl -Fo triangle_mvp.fs.spv

struct PsIn {
    float4 position : SV_Position;
    float4 color    : COLOR0;
};

float4 main(PsIn input) : SV_Target0 {
    return input.color;
}
