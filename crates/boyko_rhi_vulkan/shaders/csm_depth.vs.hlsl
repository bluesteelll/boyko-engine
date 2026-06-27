// CSM Increment 1b — Rung A: the cascade DEPTH-PASS vertex shader.
//
// Renders the scene's shadow casters from the SUN's point of view into one cascade's
// D32 depth layer, so the deferred resolve (`deferred_pbr.hlsl`) can compare each
// receiver's light-space depth against this map and `min`-combine the resulting hard
// shadow into the analytic SDF visibility term. Rung A is a SINGLE cascade (`c == 0`);
// the N-cascade select+blend is Rung B / Inc 3.
//
// This stage REUSES the foundation's instanced raster contract VERBATIM: it reads the
// SAME set-0 binding-0 per-instance `InstanceModelCol` SSBO (`instances[base_instance +
// SV_InstanceID]`) and the SAME 88-byte VERTEX push as `gbuffer_mrt.vs.hlsl`'s instanced
// arm. The ONLY differences from the gbuffer VS are:
//   * `view_proj` (push `@0`) is the CASCADE's world→light-clip matrix (`CascadeData.
//     view_proj`, column-major) — NOT the camera view-proj. The depth-pass recorder
//     pushes the cascade matrix; everything else (the instance bucket addressing, the
//     model affine layout) is identical, so the SAME instance SSBO + batches drive both.
//   * NO color / normal / eye-relative outputs: the only output is `SV_Position`. The
//     hardware writes the transformed clip-space depth; the matching `csm_depth.fs.hlsl`
//     is empty (depth-only). There is no `SV_Depth` write — the rasterizer's interpolated
//     `SV_Position.z` IS the shadow-map depth the resolve compares against.
//
// O1 MAJORNESS PIN: `view_proj` here and the `cbuffer CsmCascades` `float4x4` the resolve
// reads are BOTH column-major (DXC's default; NO `row_major` / `#pragma pack_matrix` on
// either side). `CascadeData.view_proj` is written ONCE host-side (the `resolve_csm` fit)
// and consumed by both the depth VS (here) and the resolve reprojection — identical bytes,
// identical interpretation, so the two CANNOT drift.
//
// This VS does NOT take the gbuffer VS's LEGACY (`use_model_matrix == 0`) arm: the CSM
// depth pass is ALWAYS instanced (the recorder pushes `use_model_matrix == 1`), so the
// caster's per-instance affine places its model-space vertices into world space exactly
// as the main pass does. The dummy legacy branch is kept ONLY so the push layout + the
// `instances` static reference match the gbuffer VS's pipeline-layout shape 1:1.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 csm_depth.vs.hlsl -Fo csm_depth.vs.spv

struct PushConstants {
    float4x4 view_proj;        // the CASCADE's world→light-clip transform, column-major
    float4   cam_eye;          // UNUSED by the depth pass (kept for push-layout parity)
    uint     base_instance;    // the SSBO bucket base: index instances[base_instance + SV_InstanceID]
    uint     use_model_matrix; // 1 = instanced arm (the CSM depth pass always pushes 1)
};
[[vk::push_constant]] PushConstants pc;

// Per-instance model data: the SAME 3x4 ROW-MAJOR affine the gbuffer VS reads (rows
// r0/r1/r2, each a float4 whose .xyz is a rotation/scale row and .w the translation
// component). 12 floats = 48 B per instance. STATICALLY referenced, so the pipeline layout
// MUST declare set-0 binding-0 and every draw MUST bind a valid buffer.
struct InstanceModelCol {
    float4 r0;
    float4 r1;
    float4 r2;
};
[[vk::binding(0, 0)]] StructuredBuffer<InstanceModelCol> instances;

// The vertex input matches the gbuffer raster pipeline's 40-byte stride
// (position@0 / normal@12 / color@24); only POSITION is consumed here. The field
// declaration order fixes the auto-assigned SPIR-V locations (position -> 0,
// color -> 1, normal -> 2), identical to `gbuffer_mrt.vs.hlsl`, so the SAME vertex
// buffer + the SAME `VertexAttribute` array bind unchanged.
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1 (unused by the depth pass)
    float3 normal   : NORMAL;    // SPIR-V location 2 (unused by the depth pass)
};

struct VsOut {
    float4 position : SV_Position;
};

VsOut main(VsIn input, uint instance_id : SV_InstanceID) {
    VsOut output;
    if (pc.use_model_matrix == 0u) {
        // Parity-only arm (the CSM depth pass never pushes this): transform the vertex by
        // the cascade matrix directly. Mirrors the gbuffer VS's legacy arm shape so the
        // push layout + `instances` static reference are byte-compatible.
        output.position = mul(pc.view_proj, float4(input.position, 1.0));
    } else {
        // INSTANCED arm: read the per-instance 3x4 row-major affine, place the vertex in
        // WORLD space (`m3 * pos + t`), then project by the cascade's world→light-clip
        // matrix. `m3` is the 3x3 rotation/scale; `t` the translation — IDENTICAL world
        // placement to the gbuffer VS's instanced arm, so the caster's shadow silhouette
        // matches the geometry the main pass rasterizes.
        InstanceModelCol model = instances[pc.base_instance + instance_id];
        float3x3 m3 = float3x3(model.r0.xyz, model.r1.xyz, model.r2.xyz);
        float3 t = float3(model.r0.w, model.r1.w, model.r2.w);
        float3 world = mul(m3, input.position) + t;
        output.position = mul(pc.view_proj, float4(world, 1.0));
    }
    return output;
}
