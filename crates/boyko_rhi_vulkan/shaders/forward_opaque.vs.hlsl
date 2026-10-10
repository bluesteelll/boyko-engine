// Multi-paradigm render-path plan, rung R4b: Forward render path v1 — the mesh raster VERTEX
// shader. SCOPE CUT (orchestrator-directed, v1): `Forward × Mesh` ONLY (no SDF forward-march
// yet — R-SDFFWD); NO froxel (`#ifdef FROXEL` left for R5, see `forward_opaque.fs.hlsl`); NO
// TEXTURED material path (`#ifdef TEXTURED` seam left for a later rung, mirrors
// `gbuffer_mrt.vs.hlsl`'s own T6c seam); NO motion vectors (no TAA under Forward v1 — the
// resolver's `ForwardTaaNotYetImplemented` degrade).
//
// Clones `gbuffer_mrt.vs.hlsl`'s instanced vertex path VERBATIM in SHAPE (the SAME instance
// SSBO @0 + `base_instance`/`use_model_matrix` push idiom, the SAME per-instance material SSBO
// @1 — `PerInstanceMaterial{base_color; id; _pad}`, `boyko_render::mesh_draw::PerInstanceMaterial`
// — indexed IDENTICALLY at `pc.base_instance + SV_InstanceID`), but DIVERGES on two points
// Decision 4 requires:
//
//   1. **Depth.** Deferred's raster writes a CUSTOM-LINEAR `SV_Depth` (the marcher-agreement
//      encode `md = length(eye_rel) / MESH_DEPTH_T_MAX`) because the deferred resolve
//      reconstructs world position from `gViewT`, not hardware depth. Forward's `depth` image is
//      STANDARD HARDWARE REVERSE-Z (Decision 4) — this VS emits a REAL `SV_Position.z` via
//      `pc.view_proj` (host-built by `boyko_render::view::forward_view_proj_rows`, NOT
//      `marcher_view_proj_rows`/`gbuffer_push_from_view` — a SEPARATE construction, never
//      touching the Deferred one), and the fragment shader writes NO `SV_Depth` (early-Z stays
//      live — no `SV_Depth`/`discard`/UAV in `forward_opaque.fs.hlsl`).
//   2. **Material.** `mat_id` (this shader forwards ONLY the id, NOT `PerInstanceMaterial`'s
//      `base_color` — see `forward_opaque.fs.hlsl`'s doc for why: v1's non-textured material
//      path sources albedo from `MaterialGpu.base_color` via the SAME `Materials` SSBO the
//      deferred resolve reads, keyed by this `mat_id`, not from a per-instance override).
//
// The PUSH CONSTANT layout is byte-identical to `gbuffer_mrt.vs.hlsl`'s 88-byte
// `{ float4x4 view_proj; float4 cam_eye; uint base_instance; uint use_model_matrix }`
// (`GBUFFER_PUSH_BYTES`) — ONLY the matrix CONTENT differs (reverse-Z vs custom-linear); the
// byte layout is reused deliberately so a Forward-path host recorder can reuse the existing
// push-encoding machinery with a swapped matrix-builder.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 forward_opaque.vs.hlsl -Fo forward_opaque.vs.spv

struct PushConstants {
    float4x4 view_proj;        // Forward reverse-Z proj*view, column-major (boyko_render::view::forward_view_proj_rows)
    float4   cam_eye;          // xyz = world eye position; w unused (Forward v1 is perspective-only)
    uint     base_instance;    // the SSBO bucket base: index instances[base_instance + SV_InstanceID]
    uint     use_model_matrix; // 0 = legacy arm (mul(view_proj, p)); 1 = instanced arm (per-instance model)
};
[[vk::push_constant]] PushConstants pc;

// Per-instance model data: a 3x4 ROW-MAJOR affine — byte-identical to `gbuffer_mrt.vs.hlsl`'s
// `InstanceModelCol` (the SAME host-side `InstanceModelCol` upload, no divergent layout).
struct InstanceModelCol {
    float4 r0;
    float4 r1;
    float4 r2;
};
[[vk::binding(0, 0)]] StructuredBuffer<InstanceModelCol> instances;

// Multi-paradigm render-path plan §G (Forward Set 0): the per-instance material payload —
// `boyko_render::mesh_draw::PerInstanceMaterial` (32 B: `base_color` + `id` + pad), the SAME
// struct `gbuffer_mrt.vs.hlsl`'s `PER_INSTANCE_MATERIAL` variant reads. This shader forwards
// ONLY `.id` (see the file header) — `.base_color` is read but not forwarded (the v1 non-
// textured material path sources albedo from `Materials[mat_id].base_color`, not this lane).
struct PerInstanceMaterial {
    float4 base_color;
    uint   id;
    uint3  _pad;
};
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;

// Field DECLARATION order fixes the SPIR-V vertex-input locations DXC auto-assigns (this
// codebase uses no explicit `[[vk::location]]`): position -> 0, color -> 1, normal -> 2 — the
// SAME order `gbuffer_mrt.vs.hlsl` uses, so both raster pipelines bind the IDENTICAL
// `VertexAttribute` array (position@0/normal@12/color@24, matching `boyko_render::mesh::Vertex`'s
// 64-byte stride; `color` is read but not forwarded — v1's non-textured material path does not
// shade from vertex color, see the file header).
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1 (unread by the FS this rung)
    float3 normal   : NORMAL;    // SPIR-V location 2
};

struct VsOut {
    float4 position   : SV_Position;
    float3 world_pos  : WORLDPOS;   // world-space position (Forward's FS reconstructs lighting from this)
    float3 normal     : NORMAL;     // per-vertex world normal, passed through to the fragment
    nointerpolation uint mat_id : MATID; // flat -- every pixel of a triangle reads ONE instance's material
};

// The degeneracy threshold for the instanced normal matrix — byte-identical constant to
// `gbuffer_mrt.vs.hlsl`'s M4 guard (see `inverse3x3`'s doc there for the derivation).
static const float DET_EPS = 1e-8;

// The cofactor (adjugate / determinant) inverse of a 3x3 matrix — VERBATIM copy of
// `gbuffer_mrt.vs.hlsl::inverse3x3` (M4, correct non-uniform-scale normals). Duplicated rather
// than shared via an `.hlsli` because DXC raster shaders in this codebase `#include` nothing
// (see `gbuffer_mrt.fs.hlsl`'s `MESH_DEPTH_T_MAX` doc for the same "raster shaders duplicate
// tiny math, `#include` is a compute/marcher-side idiom" precedent) — a future rung MAY promote
// this to a shared `.hlsli` once a third raster VS needs it (the O1 "wait for the real
// duplication signal" discipline).
float3x3 inverse3x3(float3x3 m) {
    float3 c0 = float3(
        m[1][1] * m[2][2] - m[1][2] * m[2][1],
        m[1][2] * m[2][0] - m[1][0] * m[2][2],
        m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    float3 c1 = float3(
        m[0][2] * m[2][1] - m[0][1] * m[2][2],
        m[0][0] * m[2][2] - m[0][2] * m[2][0],
        m[0][1] * m[2][0] - m[0][0] * m[2][1]);
    float3 c2 = float3(
        m[0][1] * m[1][2] - m[0][2] * m[1][1],
        m[0][2] * m[1][0] - m[0][0] * m[1][2],
        m[0][0] * m[1][1] - m[0][1] * m[1][0]);
    float det = m[0][0] * c0[0] + m[0][1] * c0[1] + m[0][2] * c0[2];
    float inv_det = 1.0 / det;
    return float3x3(
        c0[0] * inv_det, c1[0] * inv_det, c2[0] * inv_det,
        c0[1] * inv_det, c1[1] * inv_det, c2[1] * inv_det,
        c0[2] * inv_det, c1[2] * inv_det, c2[2] * inv_det);
}

// The determinant of a 3x3 built from the affine's three rows — VERBATIM copy of
// `gbuffer_mrt.vs.hlsl::det3x3`.
float det3x3(float3x3 m) {
    return m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
         - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
         + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
}

VsOut main(VsIn input, uint instance_id : SV_InstanceID) {
    VsOut output;
    if (pc.use_model_matrix == 0u) {
        // LEGACY arm — a merged (non-instanced) draw. `input.position` IS the world position;
        // no per-instance material exists for this arm (mirrors `gbuffer_mrt.vs.hlsl`'s legacy
        // arm: the instanced-only PM/TEXTURED pipelines never bind an empty merged-draw list).
        output.position = mul(pc.view_proj, float4(input.position, 1.0));
        output.world_pos = input.position;
        output.normal = input.normal;
        output.mat_id = 0u;
    } else {
        // INSTANCED arm — read the per-instance 3x4 row-major affine and place the vertex in
        // world space (byte-identical construction to `gbuffer_mrt.vs.hlsl`'s instanced arm).
        InstanceModelCol model = instances[pc.base_instance + instance_id];
        float3x3 m3 = float3x3(model.r0.xyz, model.r1.xyz, model.r2.xyz);
        float3 t = float3(model.r0.w, model.r1.w, model.r2.w);
        float3 world = mul(m3, input.position) + t;
        output.position = mul(pc.view_proj, float4(world, 1.0));
        output.world_pos = world;

        // M4 — correct normal under NON-UNIFORM scale via the inverse-transpose matrix, with the
        // W4 degeneracy guard (byte-identical logic to `gbuffer_mrt.vs.hlsl`'s instanced arm).
        float det = det3x3(m3);
        if (abs(det) < DET_EPS) {
            output.normal = mul(m3, input.normal);
        } else {
            float3x3 nm = transpose(inverse3x3(m3));
            output.normal = mul(nm, input.normal);
        }

        // Indexed IDENTICALLY to `instances[pc.base_instance + instance_id]` above (the SAME
        // expression, M3-proven) -- forwards ONLY `.id` (see the file header).
        PerInstanceMaterial pm = instance_materials[pc.base_instance + instance_id];
        output.mat_id = pm.id;
    }
    return output;
}
