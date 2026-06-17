// Phase-6 S0 rung-2 fragment shader: a known solid colour for every covered texel.
//
// Outputs opaque red. For an R8G8B8A8_UNORM attachment the float output
// (1.0, 0.0, 0.0, 1.0) converts to the exact bytes 0xFF 0x00 0x00 0xFF, which is
// the value the rung-2 golden checks at the (covered) centre texel — distinct
// from the rung-1 clear colour at the (uncovered) corner texel.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 triangle.fs.hlsl -Fo triangle.fs.spv

float4 main() : SV_Target0 {
    return float4(1.0, 0.0, 0.0, 1.0);
}
