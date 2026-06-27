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
// The `view_proj` is the VERTEX push the depth prepass used (it was `pc.mvp`), so the
// rasterized SV_Position.z — and thus the ORTHO marcher's `has_mesh`/`t_mesh` — is
// byte-identical to the prepass: where the quad lands is unchanged. The PERSPECTIVE step-2
// EXTENDED the push from a bare `float4x4 mvp` (64 B) to `{ float4x4 mvp; float4 cam_eye; }`
// (80 B): `cam_eye.xyz` is the world eye and `cam_eye.w` is the CAMERA MODE (0 = ortho, 1 =
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
// M1 (instanced-capable raster) WIDENS the push 80 -> 88 B and adds a set-0 binding-0
// per-instance model SSBO, behind a `use_model_matrix` wave-uniform branch:
//   * `use_model_matrix == 0` — the LEGACY arm: byte-identical to the pre-M1 VS. Every
//     existing merged-buffer draw takes this arm and rasterizes EXACTLY the same pixels
//     (the bit-identity gate). The `instances` SSBO is bound (a 1-element identity dummy)
//     but NEVER read, satisfying the pipeline layout's static reference.
//   * `use_model_matrix == 1` — the INSTANCED arm: reads a per-instance 3x4 row-major
//     affine `model` from `instances[pc.base_instance + SV_InstanceID]`, transforms the
//     vertex into world space, and recomputes `eye_rel` from the WORLD position. M1 ships
//     the capability only; no scene drives it yet (M2+).
// The normal is transformed by the affine's 3x3 part (`mul(m3, normal)`) — adequate for
// uniform scale; the inverse-transpose normal column is a later rung (M4), NOT added here.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer_mrt.vs.hlsl -Fo gbuffer_mrt.vs.spv

struct PushConstants {
    float4x4 view_proj;        // perspective (or ortho) proj*view, column-major (was `mvp`)
    float4   cam_eye;          // xyz = world eye position; w = camera mode (0 ortho, 1 perspective)
    uint     base_instance;    // the SSBO bucket base: index instances[base_instance + SV_InstanceID]
    uint     use_model_matrix; // 0 = legacy arm (mul(view_proj, p)); 1 = instanced arm (per-instance model)
};
[[vk::push_constant]] PushConstants pc;

// Per-instance model data: a 3x4 ROW-MAJOR affine (rows r0/r1/r2, each a float4 whose .xyz
// is the rotation/scale row and .w the translation component). 12 floats = 48 B per
// instance. STATICALLY referenced by the instanced arm, so the pipeline layout MUST declare
// this binding and every draw MUST bind a valid buffer (the legacy arm binds a 1-element
// identity dummy: r0=(1,0,0,0), r1=(0,1,0,0), r2=(0,0,1,0)).
struct InstanceModelCol {
    float4 r0;
    float4 r1;
    float4 r2;
};
[[vk::binding(0, 0)]] StructuredBuffer<InstanceModelCol> instances;

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

VsOut main(VsIn input, uint instance_id : SV_InstanceID) {
    VsOut output;
    output.color = input.color;
    output.cam_mode = pc.cam_eye.w;
    if (pc.use_model_matrix == 0u) {
        // LEGACY arm — BYTE-IDENTICAL to the pre-M1 VS (the bit-identity gate). The merged
        // draw rasterizes EXACTLY the same pixels: `mul(view_proj, p)` with the
        // vertex-space normal + the `cam_eye - position` ray-relative.
        output.position = mul(pc.view_proj, float4(input.position, 1.0));
        output.normal = input.normal;
        // cam_eye is constant across the primitive, so the default perspective-correct interp
        // of (cam_eye - worldpos) yields the true per-pixel (cam_eye - P) in the fragment.
        output.eye_rel = pc.cam_eye.xyz - input.position;
    } else {
        // INSTANCED arm — read the per-instance 3x4 row-major affine and place the vertex in
        // world space. `m3` is the 3x3 rotation/scale; `t` the translation.
        InstanceModelCol model = instances[pc.base_instance + instance_id];
        float3x3 m3 = float3x3(model.r0.xyz, model.r1.xyz, model.r2.xyz);
        float3 t = float3(model.r0.w, model.r1.w, model.r2.w);
        float3 world = mul(m3, input.position) + t;
        output.position = mul(pc.view_proj, float4(world, 1.0));
        // eye_rel is recomputed from the WORLD position (NOT input.position) so the
        // perspective ray-t the fragment writes reconstructs the instanced surface point.
        output.eye_rel = pc.cam_eye.xyz - world;
        // Uniform-scale-adequate normal transform (the inverse-transpose column is M4).
        output.normal = mul(m3, input.normal);
    }
    return output;
}
