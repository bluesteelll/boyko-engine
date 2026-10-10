// Multi-paradigm render-path plan, rung R4b-b (code-review follow-up): the Forward v1 sky
// BACKGROUND vertex shader — a full-screen triangle, NO vertex buffer, NO descriptor bindings.
// Drawn FIRST inside `forward_opaque`'s SAME `begin_rendering` scope (before the mesh draw loop,
// depth test/write OFF via its own pipeline — `forward_sky.fs.hlsl`'s doc), so every `lit` pixel
// gets a value before opaque geometry draws over the pixels it covers.
//
// VERBATIM copy of `fullscreen_sample.vs.hlsl`'s "oversized triangle" trick (its own doc has the
// full derivation), minus the UV output — `forward_sky.fs.hlsl` reconstructs a per-pixel camera
// ray from `SV_Position` + the Camera UBO (`ray_gen.hlsli`), not a texture sample, so no UV is
// needed here.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 forward_sky.vs.hlsl -Fo forward_sky.vs.spv

float4 main(uint vid : SV_VertexID) : SV_Position {
    // x = -1, 3, -1 ; y = -1, -1, 3 → a triangle covering all of NDC [-1, 1]^2.
    float2 pos = float2((vid == 1) ? 3.0 : -1.0, (vid == 2) ? 3.0 : -1.0);
    return float4(pos, 0.0, 1.0);
}
