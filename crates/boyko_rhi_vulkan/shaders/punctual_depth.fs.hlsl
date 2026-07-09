// Shadow Phase 5 Increment 2 (POINT cube) — the punctual POINT DEPTH-PASS fragment shader.
//
// Writes `SV_Depth = saturate(length(world - light_pos) * inv_range)` — the LINEAR RADIAL
// distance from the point light to this fragment, normalized by the light range. Unlike the
// cascade / spot depth pass (which leaves an EMPTY FS and lets the rasterizer store the
// perspective NDC-z), a POINT cube map must store a value that is COMPARABLE ACROSS ALL SIX
// faces: the radial distance is face-independent, so the resolve can major-axis-select one face,
// fetch the stored normalized distance, and compare it against the receiver's own
// `length(P - light_pos) * inv_range` (matching this exact expression) — the standard cube
// shadow-map distance compare. The PCF comparison sampler is `LessOrEqual` (shared with the spot
// path); for radial distance `ref = dist/range` vs stored `dist/range` it has the SAME sense (a
// receiver farther than the stored occluder is in shadow).
//
// `light_pos` / `inv_range` ride in the DEAD `cam_eye@64` push lane (the depth pass has no camera
// eye): `cam_eye.xyz = light_pos`, `cam_eye.w = inv_range`. The recorder stamps that lane per point
// face; the lane is otherwise unused by the depth pass, so this needs NO push-layout change. The
// pipeline layout's push range covers `VERTEX | FRAGMENT` so this stage can read it.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 punctual_depth.fs.hlsl -Fo punctual_depth.fs.spv

struct PushConstants {
    float4x4 view_proj;        // UNUSED by the FS (the VS projects); kept for push-layout parity
    float4   cam_eye;          // xyz = light_pos, w = inv_range
    uint     base_instance;    // UNUSED by the FS
    uint     use_model_matrix; // UNUSED by the FS
};
[[vk::push_constant]] PushConstants pc;

struct VsOut {
    float4 position : SV_Position;
    float3 world    : WORLDPOS;  // the interpolated world position (the VS forwarded it)
};

// The depth output: the linear radial distance, saturated to the Vulkan [0,1] depth range. The
// `inv_range` normalizer maps `dist == range` to 1.0, matching the resolve's compare scale.
float main(VsOut input) : SV_Depth {
    float3 light_pos = pc.cam_eye.xyz;
    float inv_range = pc.cam_eye.w;
    float dist = length(input.world - light_pos);
    return saturate(dist * inv_range);
}
