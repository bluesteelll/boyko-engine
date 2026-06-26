// Render P5-r0 mesh-MRT G-buffer PRODUCER vertex shader.
//
// Pass A's mesh raster was a depth-only prepass (a throwaway color + the D32 depth).
// P5-r0 upgrades it into a 3-MRT G-buffer producer so a yielded mesh pixel
// (post-r1) has a real albedo/normal/material fragment to stand on. This stage feeds
// the matching `gbuffer_mrt.fs.hlsl`.
//
// The vertex buffer carries a PER-VERTEX world normal now: POSITION at offset 0
// (R32G32B32_SFLOAT, location 0) + NORMAL at offset 12 (R32G32B32_SFLOAT, location 2)
// + COLOR at offset 24 (R32G32B32A32_SFLOAT, location 1), 40-byte stride. The previous
// rung was the fronto-parallel +Z quad, so the VS hardcoded `(0, 0, 1)`; the multi-object
// mesh step gives each vertex its face's outward normal, so the VS now passes the
// per-vertex `input.normal` through to the G-buffer fragment unchanged.
//
// The MVP is the VERTEX push the depth prepass used (`pc.mvp`), so the rasterized
// SV_Position.z — and thus the ORTHO marcher's `has_mesh`/`t_mesh` — is byte-identical
// to the prepass: where the quad lands is unchanged. The PERSPECTIVE step-2 EXTENDS the
// push from a bare `float4x4 mvp` (64 B) to `{ float4x4 mvp; float4 cam_eye; }` (80 B):
// `cam_eye.xyz` is the world eye and `cam_eye.w` is the CAMERA MODE (0 = ortho, 1 =
// perspective). The VS forwards `eye_rel = cam_eye.xyz - position` to the fragment, which
// — under perspective — writes the EUCLIDEAN ray-t `length(eye_rel) / T_MAX` via SV_Depth
// so the marcher's UNIT-`rd` decode `t_mesh = md * T_MAX` reconstructs the true mesh
// surface point. Under ortho the fragment ignores `eye_rel` and keeps SV_Position.z (the
// axial `(CAM_Z - z) / T_MAX`), so the ORTHO depth — and the 41 ortho goldens — are
// byte-identical to step 1.
//
// `eye_rel` is a PERSPECTIVE-CORRECT (default) varying, NOT `noperspective`: `cam_eye` is
// constant across the triangle, so the perspective-correct interpolation of `cam_eye -
// worldpos` reconstructs the true per-pixel `cam_eye - P_pixel` (the same point the
// marcher's ray through that pixel reaches).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer_mrt.vs.hlsl -Fo gbuffer_mrt.vs.spv

struct PushConstants {
    float4x4 mvp;       // perspective (or ortho) proj*view, column-major
    float4   cam_eye;   // xyz = world eye position; w = camera mode (0 ortho, 1 perspective)
};
[[vk::push_constant]] PushConstants pc;

// Field DECLARATION order fixes the SPIR-V vertex-input locations DXC auto-assigns
// (this codebase uses no explicit `[[vk::location]]`): position -> 0, color -> 1,
// normal -> 2. The vertex BUFFER offsets are independent of this order and are bound by
// the pipeline's `VertexAttribute` array (position@0, normal@12, color@24, 40-byte stride).
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1
    float3 normal   : NORMAL;    // SPIR-V location 2
};

struct VsOut {
    float4 position : SV_Position;
    float4 color    : COLOR0;       // LINEAR base color, passed through to the fragment
    float3 normal   : NORMAL;       // per-vertex world normal, passed through to the fragment
    float3 eye_rel  : WORLDDIST;    // cam_eye.xyz - world position (perspective-correct interp)
    float  cam_mode : CAMMODE;      // 0 = ortho (use SV_Position.z), 1 = perspective (use eye_rel)
};

VsOut main(VsIn input) {
    VsOut output;
    output.position = mul(pc.mvp, float4(input.position, 1.0));
    output.color = input.color;
    output.normal = input.normal;
    // cam_eye is constant across the primitive, so the default perspective-correct interp
    // of (cam_eye - worldpos) yields the true per-pixel (cam_eye - P) in the fragment.
    output.eye_rel = pc.cam_eye.xyz - input.position;
    output.cam_mode = pc.cam_eye.w;
    return output;
}
