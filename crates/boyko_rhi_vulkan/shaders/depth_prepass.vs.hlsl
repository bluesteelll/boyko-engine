// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-only PRE-PASS vertex
// shader. Recorded FIRST among the Forward pass sequence under `ForwardPlus` (Decision 4's
// EQUAL-depth early-Z contract): writes the SAME hardware reverse-Z `forward_depth` image
// `forward_opaque.vs/fs.hlsl` renders into, so `forward_opaque`'s later `VK_COMPARE_OP_EQUAL`
// pass (depth-write OFF) can hardware-early-Z-reject every fragment this prepass did not own.
//
// A POSITION-ONLY SUBSET of `forward_opaque.vs.hlsl`'s instanced arm: the SAME instance SSBO
// @0 + `base_instance`/`use_model_matrix` push idiom + reverse-Z `pc.view_proj` rows
// (`boyko_render::view::forward_view_proj_rows`), but exports NO normal/mat_id interpolant
// (`forward_opaque.fs.hlsl`'s inline shade never runs in this pass — there is no fragment
// shader work needing them) and references neither `Camera` nor `instance_materials` (this
// pipeline's Set 0 is REUSED verbatim from `forward_opaque`'s own UNIFIED `forward_layout0`
// (rung R5 code-review fix: ONE Set-0 layout shared by every Forward-family pipeline) — the SAME
// "shader references a subset of what its layout declares" idiom `forward_sky_pipeline` already
// establishes, so no new bind-group layout is needed).
//
// POSITION IDENTITY (rung R5 code-review audit): this VS's instanced-arm transform expression —
// `instances[pc.base_instance + instance_id]` -> `mul(m3, input.position) + t` ->
// `mul(pc.view_proj, float4(world, 1.0))` — is TOKEN-FOR-TOKEN identical to
// `forward_opaque.vs.hlsl`'s own instanced arm (that file's position computation, verbatim minus
// the normal/mat_id lanes this depth-only pass omits), reading the SAME `instances` SSBO at the
// SAME index with the SAME 88-byte push layout. `forward_opaque`'s EQUAL-depth test therefore
// compares against a value derived from the IDENTICAL clip-space expression it itself computes —
// required for the EQUAL test to ever pass (Decision 4).
//
// The PUSH CONSTANT layout is byte-identical to `forward_opaque.vs.hlsl`'s 88-byte
// `{ float4x4 view_proj; float4 cam_eye; uint base_instance; uint use_model_matrix }`
// (`GBUFFER_PUSH_BYTES`) — `cam_eye` is declared but unread (this VS needs only `view_proj` +
// the two `uint`s), kept for byte-layout parity so the host push-encoding machinery
// (`forward_gbuffer_push_from_view`) is reused verbatim across both pipelines.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 depth_prepass.vs.hlsl -Fo depth_prepass.vs.spv

struct PushConstants {
    float4x4 view_proj;        // Forward reverse-Z proj*view, column-major (boyko_render::view::forward_view_proj_rows)
    float4   cam_eye;          // unread by this position-only pass; kept for push-byte-layout parity
    uint     base_instance;    // the SSBO bucket base: index instances[base_instance + SV_InstanceID]
    uint     use_model_matrix; // 0 = legacy arm (mul(view_proj, p)); 1 = instanced arm (per-instance model)
};
[[vk::push_constant]] PushConstants pc;

// Per-instance model data: a 3x4 ROW-MAJOR affine — byte-identical to `forward_opaque.vs.hlsl`'s
// `InstanceModelCol` (the SAME host-side upload; this pass reads the identical SSBO at Set 0
// binding 0).
struct InstanceModelCol {
    float4 r0;
    float4 r1;
    float4 r2;
};
[[vk::binding(0, 0)]] StructuredBuffer<InstanceModelCol> instances;

// Field DECLARATION order fixes the SPIR-V vertex-input locations DXC auto-assigns — the SAME
// order `forward_opaque.vs.hlsl`/`gbuffer_mrt.vs.hlsl` use (position@0/normal@12/color@24 in the
// `boyko_render::mesh::Vertex` 64-byte stride), so this pipeline binds the IDENTICAL
// `VertexAttribute` array — normal/color are declared (matching the shared vertex layout) but
// unread by this position-only pass.
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1 (unread)
    float3 normal   : NORMAL;    // SPIR-V location 2 (unread)
};

float4 main(VsIn input, uint instance_id : SV_InstanceID) : SV_Position {
    if (pc.use_model_matrix == 0u) {
        // LEGACY arm — a merged (non-instanced) draw. `input.position` IS the world position.
        return mul(pc.view_proj, float4(input.position, 1.0));
    }
    // INSTANCED arm — read the per-instance 3x4 row-major affine and place the vertex in world
    // space (byte-identical construction to `forward_opaque.vs.hlsl`'s instanced arm's position
    // math; the normal/mat_id computation there is omitted — this pass writes depth only).
    InstanceModelCol model = instances[pc.base_instance + instance_id];
    float3x3 m3 = float3x3(model.r0.xyz, model.r1.xyz, model.r2.xyz);
    float3 t = float3(model.r0.w, model.r1.w, model.r2.w);
    float3 world = mul(m3, input.position) + t;
    return mul(pc.view_proj, float4(world, 1.0));
}
