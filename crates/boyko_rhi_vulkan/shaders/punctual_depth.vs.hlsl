// Shadow Phase 5 Increment 2 (POINT cube) — the punctual POINT DEPTH-PASS vertex shader.
//
// Renders the scene's shadow casters from a POINT light's point of view into one cube-face
// atlas layer, so the deferred resolve (`deferred_pbr.hlsl`) can do a major-axis face-select +
// a LINEAR-DISTANCE compare against this map and multiply the resulting hard shadow into the
// point light's contribution. A POINT consumes SIX contiguous layers (the ±X/±Y/±Z cube faces);
// the depth pass loops the six faces, each with its own 90°-FOV `view_proj` pushed at `@0`.
//
// This stage REUSES the foundation's instanced raster contract VERBATIM (the SAME set-0
// binding-0 `InstanceModelCol` SSBO + the SAME 88-byte VERTEX push as `csm_depth.vs.hlsl`).
// The ONLY differences from the cascade depth VS are:
//   * It ALSO forwards the WORLD position to the fragment (a varying `world`), because the
//     matching `punctual_depth.fs.hlsl` writes `SV_Depth = saturate(length(world - light_pos)
//     * inv_range)` — the LINEAR RADIAL distance from the light, NOT the perspective NDC-z.
//     (`csm_depth.fs` is empty and the rasterizer's interpolated NDC-z is the depth; a cube
//     face needs the true distance so all six faces share ONE comparison scale.)
//   * The fragment reads the light's `{position, inv_range}` from the currently-DEAD
//     `cam_eye@64` push lane (`cam_eye.xyz = light_pos`, `cam_eye.w = inv_range`) — NO
//     push-layout change. The depth-pass recorder stamps that lane per point face.
//
// O1 MAJORNESS PIN: `view_proj` here and the `cbuffer ShadowAtlas` `float4x4` the resolve reads
// are BOTH column-major (DXC default; NO `row_major`). The host fit writes each face's
// `view_proj` ONCE and both the depth VS (here) and the resolve read identical bytes — they
// cannot drift. (The depth FS does NOT use `view_proj` for the stored depth — it uses the
// world-space distance — so only the RASTER footprint comes from `view_proj`; the COMPARE scale
// is the shared linear distance.)
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 punctual_depth.vs.hlsl -Fo punctual_depth.vs.spv

struct PushConstants {
    float4x4 view_proj;        // this cube FACE's world->light-clip transform, column-major
    float4   cam_eye;          // xyz = light_pos, w = inv_range (the FS distance compare)
    uint     base_instance;    // the SSBO bucket base: index instances[base_instance + SV_InstanceID]
    uint     use_model_matrix; // 1 = instanced arm (the punctual depth pass always pushes 1)
};
[[vk::push_constant]] PushConstants pc;

// Per-instance model data: the SAME 3x4 ROW-MAJOR affine the gbuffer / cascade depth VS read.
struct InstanceModelCol {
    float4 r0;
    float4 r1;
    float4 r2;
};
[[vk::binding(0, 0)]] StructuredBuffer<InstanceModelCol> instances;

// The vertex input matches the gbuffer raster pipeline's 40-byte stride (position@0 / normal@12 /
// color@24); only POSITION is consumed. The field declaration order fixes the auto-assigned SPIR-V
// locations (position -> 0, color -> 1, normal -> 2), identical to `csm_depth.vs.hlsl`, so the SAME
// vertex buffer + the SAME `VertexAttribute` array bind unchanged.
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1 (unused)
    float3 normal   : NORMAL;    // SPIR-V location 2 (unused)
};

struct VsOut {
    float4 position : SV_Position;
    float3 world    : WORLDPOS;  // forwarded to the FS for the radial-distance compare
};

VsOut main(VsIn input, uint instance_id : SV_InstanceID) {
    VsOut output;
    if (pc.use_model_matrix == 0u) {
        // Parity-only arm (the punctual depth pass never pushes this): transform the vertex by the
        // face matrix directly + forward the model-space position as the "world" varying. Mirrors
        // the cascade depth VS's legacy arm shape so the push layout + `instances` static reference
        // are byte-compatible.
        output.position = mul(pc.view_proj, float4(input.position, 1.0));
        output.world = input.position;
    } else {
        // INSTANCED arm: read the per-instance 3x4 row-major affine, place the vertex in WORLD
        // space (`m3 * pos + t`), then project by this cube face's world->light-clip matrix.
        // IDENTICAL world placement to the gbuffer / cascade depth VS, so the caster's shadow
        // silhouette matches the geometry the main pass rasterizes. The world position is forwarded
        // so the FS computes the true light->fragment distance.
        InstanceModelCol model = instances[pc.base_instance + instance_id];
        float3x3 m3 = float3x3(model.r0.xyz, model.r1.xyz, model.r2.xyz);
        float3 t = float3(model.r0.w, model.r1.w, model.r2.w);
        float3 world = mul(m3, input.position) + t;
        output.position = mul(pc.view_proj, float4(world, 1.0));
        output.world = world;
    }
    return output;
}
