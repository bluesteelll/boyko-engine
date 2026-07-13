// Multi-paradigm render-path plan, rung R8: the VisibilityBuffer mesh raster VERTEX shader.
// A POSITION-ONLY clone of `depth_prepass.vs.hlsl`'s instanced arm (the SAME instance-SSBO @0 +
// `base_instance`/`use_model_matrix` push idiom + reverse-Z `pc.view_proj` rows,
// `boyko_render::view::forward_view_proj_rows`, Decision 4), with TWO differences:
//
//   1. **Instance row shape.** Reads `VbInstanceRow` (64 B: the SAME leading 48-byte 3x4
//      row-major affine as `InstanceModelCol`, plus an appended `mesh_id` lane at offset 48 —
//      `boyko_render::instance_model::VbInstanceRow`) instead of the 48-byte `InstanceModelCol`.
//      This VS does not itself read `mesh_id` (the compute fetch, `vb_geom_fetch.hlsli`, reads
//      it back from the SAME SSBO by `instance_id` — no VS export needed).
//   2. **Flat instance-id export.** `global_instance_id = pc.base_instance + SV_InstanceID` is
//      exported as a `nointerpolation` flat interpolant (Decision 9: `SV_InstanceID` is a
//      VS-only system value with no guaranteed FS-side read, so the id is threaded flat instead
//      of recomputed in the fragment stage from a bare push + a nonexistent FS `SV_InstanceID`).
//
// NO jitter this rung (TAA is capped off under VB v1 — `RenderPathDegrade::VbTaaNotYetImplemented`
// — so there is no supersampled history to align against yet).
//
// The PUSH CONSTANT layout is byte-identical to `forward_opaque.vs.hlsl`'s / `depth_prepass.vs.hlsl`'s
// 88-byte `{ float4x4 view_proj; float4 cam_eye; uint base_instance; uint use_model_matrix }`
// (`GBUFFER_PUSH_BYTES`) — `cam_eye` is unread by this VS, kept for push-byte-layout parity so the
// SAME host push-encoding machinery is reused verbatim.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_raster.vs.hlsl -Fo vb_raster.vs.spv

struct PushConstants {
    float4x4 view_proj;        // reverse-Z proj*view, column-major (boyko_render::view::forward_view_proj_rows)
    float4   cam_eye;          // unread by this position-only pass; kept for push-byte-layout parity
    uint     base_instance;    // the SSBO bucket base: index instances[base_instance + SV_InstanceID]
    uint     use_model_matrix; // 0 = legacy arm (mul(view_proj, p)); 1 = instanced arm (per-instance model)
};
[[vk::push_constant]] PushConstants pc;

// Decision 0's VB-path instance row (64 B) — byte-identical leading bytes to `InstanceModelCol`
// (offset 0..48), plus the appended `mesh_id` lane (offset 48) and a 12-byte pad (offset 52..64,
// std430 stability). This VS reads ONLY `r0`/`r1`/`r2` — `mesh_id`/`_pad` are declared for byte
// layout parity but never referenced here (the compute fetch reads them back by `instance_id`).
struct VbInstanceRow {
    float4 r0;
    float4 r1;
    float4 r2;
    uint   mesh_id;
    uint3  _pad;
};
[[vk::binding(0, 0)]] StructuredBuffer<VbInstanceRow> instances;

// Field DECLARATION order fixes the SPIR-V vertex-input locations DXC auto-assigns — the SAME
// order every other raster VS in this codebase uses (position@0/normal@12/color@24 in the
// `boyko_render::mesh::Vertex` 64-byte stride); normal/color are declared for `VertexAttribute`
// parity but unread by this position-only pass.
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1 (unread)
    float3 normal   : NORMAL;    // SPIR-V location 2 (unread)
};

struct VsOut {
    float4 position : SV_Position;
    nointerpolation uint instance_id : IID; // pc.base_instance + SV_InstanceID (Decision 9)
};

VsOut main(VsIn input, uint instance_id : SV_InstanceID) {
    VsOut output;
    if (pc.use_model_matrix == 0u) {
        // LEGACY arm — a merged (non-instanced) draw. `input.position` IS the world position;
        // no per-instance row exists for this arm (mirrors every other raster VS's legacy arm).
        output.position = mul(pc.view_proj, float4(input.position, 1.0));
        output.instance_id = 0u;
        return output;
    }
    // INSTANCED arm — read the per-instance 3x4 row-major affine and place the vertex in world
    // space (byte-identical construction to `forward_opaque.vs.hlsl`'s instanced arm).
    VbInstanceRow model = instances[pc.base_instance + instance_id];
    float3x3 m3 = float3x3(model.r0.xyz, model.r1.xyz, model.r2.xyz);
    float3 t = float3(model.r0.w, model.r1.w, model.r2.w);
    float3 world = mul(m3, input.position) + t;
    output.position = mul(pc.view_proj, float4(world, 1.0));
    output.instance_id = pc.base_instance + instance_id;
    return output;
}
