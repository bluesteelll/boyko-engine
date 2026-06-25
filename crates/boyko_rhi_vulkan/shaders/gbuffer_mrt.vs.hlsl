// Render P5-r0 mesh-MRT G-buffer PRODUCER vertex shader.
//
// Pass A's mesh raster was a depth-only prepass (a throwaway color + the D32 depth).
// P5-r0 upgrades it into a 3-MRT G-buffer producer so a yielded mesh pixel
// (post-r1) has a real albedo/normal/material fragment to stand on. This stage feeds
// the matching `gbuffer_mrt.fs.hlsl`.
//
// The vertex buffer is the rung-3/4 layout REUSED byte-for-byte: POSITION at offset 0
// (R32G32B32_SFLOAT) + COLOR at offset 12 (R32G32B32A32_SFLOAT), 28-byte stride. P5-r0
// adds NO vertex attribute — the mesh is a fronto-parallel camera-facing quad (the
// hybrid scene's `quad_vertices`), so its world normal is the constant `(0, 0, 1)`
// (the same constant `gbuffer.vs.hlsl` bakes for its +Z quad). Emitting the constant
// here keeps the vertex buffer + both raster drivers' attribute layout unchanged; a
// per-vertex world normal (curved meshes) is a charted follow-up that reformats the
// buffer.
//
// The MVP is the SAME 64-byte VERTEX push the depth prepass used (`pc.mvp`), so the
// rasterized depth — and thus the marcher's `has_mesh`/`t_mesh` — is byte-identical to
// the prepass: P5-r0 changes only what the fragment WRITES, never where the quad lands.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer_mrt.vs.hlsl -Fo gbuffer_mrt.vs.spv

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
    float4 color    : COLOR0;     // LINEAR base color, passed through to the fragment
    float3 normal   : NORMAL;     // world normal (the fronto-parallel quad's +Z constant)
};

VsOut main(VsIn input) {
    VsOut output;
    output.position = mul(pc.mvp, float4(input.position, 1.0));
    output.color = input.color;
    output.normal = float3(0.0, 0.0, 1.0);
    return output;
}
