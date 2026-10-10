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
// M4 (correct non-uniform-scale normals) replaces the instanced arm's `mul(m3, normal)` —
// which only stays perpendicular to the surface under rotation + UNIFORM scale — with the
// inverse-transpose normal matrix `transpose(inverse3x3(m3))`, computed PER-VERTEX from the
// model column the instanced arm already reads. NO second SSBO binding and NO normal column:
// the `InstanceModelCol` stays 48 B (a future CSM depth pass reads only the 48 B model data,
// the W2 goal), and a 3x3 inverse per vertex is negligible at this scale. A degeneracy guard
// (`abs(det) < DET_EPS`) falls back to the M3 `mul(m3, normal)` so a zero-scale / mirror-
// singular transform cannot poison the normal MRT with NaN/Inf. The LEGACY arm
// (`use_model_matrix == 0`) is UNCHANGED (the bit-identity gate).
//
// Rung-3b MOTION_VECTORS variant (opt-in, compiled with `-D MOTION_VECTORS=1`): adds a
// per-object + camera motion vector as the fragment's 4th MRT. This VS additionally reads
// (a) a SECOND per-instance model ring `prev_instances` (binding 1, byte-identical 48 B
// layout to `instances`) holding LAST frame's transforms, and (b) a `MotionCam` UBO
// (binding 2) carrying the current + previous marcher-aligned view-proj. It forwards the
// current and previous CLIP positions (`cur_clip`/`prev_clip`) to the fragment, which
// divides both and writes `Δuv`. EVERYTHING new is gated under `#ifdef MOTION_VECTORS`, so
// the base (no-define) compile is byte-frozen — the `gbuffer_mrt.vs.spv` golden is
// untouched (the Rung-3b step-5 byte-identity gate).
//
// Asset-streaming plan F8 PER_INSTANCE_MATERIAL variant (opt-in, compiled with
// `-D PER_INSTANCE_MATERIAL=1`): reads a per-instance material PAYLOAD SSBO (binding 1,
// VERTEX) at the SAME `pc.base_instance + instance_id` index the model-matrix arm
// already uses, and forwards it flat (`nointerpolation`) to the fragment: the id, which
// packs into `gNormal.BA` instead of the compile-time default id (unchanged from F8),
// and (F8+, owner: material-drives-albedo-too) the material's `base_color`, which the
// PM fragment sources `gAlbedo` from instead of the mesh vertex color. EVERYTHING new
// is gated under `#ifdef PER_INSTANCE_MATERIAL`, so the base (no-define) compile is
// byte-frozen — the `gbuffer_mrt.vs.spv` golden is untouched.
//
// F8-mv (opt-in, compiled with BOTH `-D MOTION_VECTORS=1 -D PER_INSTANCE_MATERIAL=1`):
// the two variants above ARE compiled together to produce the combined `gbuffer_mrt_mvpm`
// pair. Their only interface collision is set-0 binding 1 (`prev_instances` under
// MOTION_VECTORS vs. `instance_materials` under PER_INSTANCE_MATERIAL); resolved by a
// nested `#if defined(MOTION_VECTORS)` inside the `PER_INSTANCE_MATERIAL` block below that
// moves `instance_materials` to binding 3 ONLY on the combined compile — the PM-only compile
// keeps binding 1 unchanged, so `gbuffer_mrt_pm.{vs,fs}.spv` stays byte-frozen too.
//
// Textured-PBR rung T6c TEXTURED variant (opt-in, compiled with `-D TEXTURED=1`): an
// INDEPENDENT #ifdef axis from PER_INSTANCE_MATERIAL/MOTION_VECTORS — NEVER compiled
// together with either (T6c plan Decision D4 / locked decision 1: TEXTURED × MOTION_VECTORS
// is never built; under active temporal denoise a textured material renders base_color/
// scalar through the MV/mvpm pipeline instead, with a one-time host-side warning). Reads a
// per-instance TEXTURED material PAYLOAD SSBO (set 0, binding 1, VERTEX — its OWN dedicated
// binding, indexed IDENTICALLY to `instances` @0, the SAME `pc.base_instance +
// SV_InstanceID` expression the model-matrix arm already uses) and forwards it FLAT to the
// fragment: `base_color`, the material id, the five bindless texture slots, and the
// fallback `metallic`/`roughness` scalars. ADDITIONALLY reads the vertex `uv`/`tangent`
// attributes (declared 4th/5th, so DXC auto-assigns them SPIR-V locations 3/4 — see the
// field-order note below) to build the tangent-space basis the fragment normal-maps with:
// `world_T = normalize(mul(m3, tangent.xyz))` — the PLAIN model 3x3 (`m3`, the SAME matrix
// the instanced arm already builds for the position/eye_rel transform — NOT the
// inverse-transpose normal matrix `nm` M4 computes), the glTF/Mikktspace convention —
// forwarded as a perspective-correct varying alongside `uv` and the handedness sign
// `tangent.w`. EVERYTHING new is gated under `#ifdef TEXTURED`, so the base (no-`-D`)
// compile is byte-frozen — the `gbuffer_mrt.vs.spv` golden is untouched.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer_mrt.vs.hlsl -Fo gbuffer_mrt.vs.spv
//   (MOTION_VECTORS variant: add `-D MOTION_VECTORS=1 -Fo gbuffer_mrt_mv.vs.spv`)
//   (PER_INSTANCE_MATERIAL variant: add `-D PER_INSTANCE_MATERIAL=1 -Fo gbuffer_mrt_pm.vs.spv`)
//   (F8-mv combined variant: add `-D MOTION_VECTORS=1 -D PER_INSTANCE_MATERIAL=1
//       -Fo gbuffer_mrt_mvpm.vs.spv`)
//   (TEXTURED variant: add `-D TEXTURED=1 -Fo gbuffer_mrt_tex.vs.spv`)

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

#ifdef MOTION_VECTORS
// Rung-3b motion-vector inputs (opt-in). `prev_instances` is the LAST-frame per-instance
// model ring (byte-identical 48 B layout to `instances`, uploaded via the FIF-parallel
// prev-instance ring); the instanced arm reads slot `base_instance + SV_InstanceID` to
// reconstruct the object's PREVIOUS world position. `MotionCam` carries the current and
// previous marcher-aligned view-proj (column-major, the SAME convention `pc.view_proj`
// uses — `mc_cur_view_proj` is bit-equal to `pc.view_proj` by construction, but sourcing
// both clip positions from ONE UBO keeps a static object's `cur_clip == prev_clip` exact,
// so `Δuv == 0` for a still scene). Declared ENTIRELY under `#ifdef MOTION_VECTORS`, so the
// base variant never references bindings 1/2 — its layout stays the single-SSBO gate.
[[vk::binding(1, 0)]] StructuredBuffer<InstanceModelCol> prev_instances;
[[vk::binding(2, 0)]] cbuffer MotionCam {
    float4x4 mc_cur_view_proj;   // current marcher-aligned proj*view (== pc.view_proj)
    float4x4 mc_prev_view_proj;  // last frame's marcher-aligned proj*view
};
#endif

#ifdef PER_INSTANCE_MATERIAL
// Asset-streaming plan F8+ (owner: material-drives-albedo-too): the per-instance
// material PAYLOAD — the FINAL, already CPU OOB-clamped material slot (F8 §4.2) PLUS
// that slot's LINEAR `base_color` (the Rust host mirror is
// `boyko_render::mesh_draw::PerInstanceMaterial`; a `float4` cannot straddle a
// 16-byte boundary under HLSL structured-buffer packing, so `id` immediately
// following `base_color` and `_pad` filling the rest of the second 16-byte lane
// produces the SAME 32-byte element stride host- and device-side).
struct PerInstanceMaterial {
    float4 base_color;
    uint   id;
    uint3  _pad;
};
// Set 0, binding 1 (VERTEX) on the PM-only compile. Indexed IDENTICALLY to `instances`
// @0 — the SAME `pc.base_instance + SV_InstanceID` expression the model-matrix arm
// already uses (M3-proven), so `instance_materials[i]` always names the SAME instance
// `instances[i]` does. Declared ENTIRELY under this `#ifdef`, so the base variant never
// references binding 1 — its layout stays the single-SSBO gate.
//
// F8-mv: on the COMBINED compile (`MOTION_VECTORS` also defined), binding 1 is already
// taken by `prev_instances` above, so `instance_materials` moves to binding 3 (the next
// free slot after `MotionCam` @2) — ONLY this nested branch differs between the PM-only
// and combined compiles; the `#else` arm below is byte-identical to the PM-only source,
// so `gbuffer_mrt_pm.vs.spv` is untouched by this change.
#if defined(MOTION_VECTORS)
[[vk::binding(3, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;
#else
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;
#endif
#endif

#ifdef TEXTURED
// Textured-PBR rung T6c: the per-instance TEXTURED material PAYLOAD — PerInstanceMaterial's
// `base_color`/id PLUS the resolved row's five bindless texture slots PLUS the FALLBACK
// `metallic`/`roughness` scalars the fragment's gPbr override uses when the metal-rough
// slot is `0` (the Rust host mirror is `boyko_render::mesh_draw::PerInstanceMaterialTex`).
// NO explicit pad field: the eight trailing scalars (six `uint` + two `float`) run
// contiguously from offset 16 to 48 with none straddling a 16-byte lane, so the natural
// HLSL struct size is already 48 bytes — the SAME element stride the host uploads.
struct PerInstanceMaterialTex {
    float4 base_color;
    uint   material_id;
    uint   albedo;
    uint   normal;
    uint   metal_rough;
    uint   ao;
    uint   emissive;
    float  metallic;
    float  roughness;
};
// Set 0, binding 1 (VERTEX) on the TEXTURED compile — its OWN dedicated binding, NOT
// PER_INSTANCE_MATERIAL's `instance_materials` above (TEXTURED is an INDEPENDENT #ifdef
// axis, never compiled together with PER_INSTANCE_MATERIAL or MOTION_VECTORS — T6c plan
// Decision D4). Indexed IDENTICALLY to `instances` @0 — the SAME `pc.base_instance +
// SV_InstanceID` expression the model-matrix arm already uses (M3-proven).
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterialTex> instance_materials_tex;
#endif

// Field DECLARATION order fixes the SPIR-V vertex-input locations DXC auto-assigns
// (this codebase uses no explicit `[[vk::location]]`): position -> 0, color -> 1,
// normal -> 2. The vertex BUFFER offsets are independent of this order and are bound by
// the pipeline's `VertexAttribute` array (position@0, normal@12, color@24, 40-byte stride).
struct VsIn {
    float3 position : POSITION;  // SPIR-V location 0
    float4 color    : COLOR0;    // SPIR-V location 1
    float3 normal   : NORMAL;    // SPIR-V location 2
#ifdef TEXTURED
    // Textured-PBR rung T6c: declared 4th/5th, so DXC auto-assigns locations 3/4 (the
    // field-order rule above) — matching `boyko_render::mesh::Vertex`'s uv@40/tangent@48
    // vertex-buffer offsets, which the TEXTURED pipeline's `VertexAttribute` array binds.
    float2 uv      : TEXCOORD0;  // SPIR-V location 3
    float4 tangent : TANGENT;    // SPIR-V location 4 (xyz = unit tangent, w = handedness)
#endif
};

struct VsOut {
    float4 position : SV_Position;
    float4 color    : COLOR0;       // LINEAR base color, passed through to the fragment
    float3 normal   : NORMAL;       // per-vertex world normal, passed through to the fragment
    float3 eye_rel  : WORLDDIST;    // cam_eye.xyz - world position (perspective-correct interp)
    float  cam_mode : CAMMODE;      // 0 = ortho (use SV_Position.z), 1 = perspective (use eye_rel)
#ifdef PER_INSTANCE_MATERIAL
    // flat (per-primitive constant) — never interpolated, so every pixel of a triangle
    // reads the SAME instance's material id.
    nointerpolation uint mat_id : MATID;
    // F8+ (owner: material-drives-albedo-too): the SAME instance's LINEAR base_color,
    // flat like `mat_id` — the PM fragment's `gAlbedo` source.
    nointerpolation float3 mat_albedo : MATALBEDO;
#endif
#ifdef MOTION_VECTORS
    // Marcher-aligned clip positions (NOT SV_Position — passed as perspective-correct
    // varyings so the fragment divides BOTH through the identical interpolation path;
    // a static object then has `cur_clip == prev_clip` per pixel ⇒ Δuv == 0 exactly).
    float4 cur_clip  : CURCLIP;     // mc_cur_view_proj  * cur_world
    float4 prev_clip : PREVCLIP;    // mc_prev_view_proj * prev_world
#endif
#ifdef TEXTURED
    // uv/world_T/tex_w are perspective-correct (default) — T6c plan Decision D2/D3 (matches
    // `normal`/`eye_rel` above; the fragment interpolates them the SAME way it always has).
    float2 uv       : TEXUV;
    float3 world_T  : TEXTANGENT;   // tangent transformed by the PLAIN model 3x3 (m3)
    float  tex_w    : TEXHAND;      // tangent.w, the bitangent handedness sign
    // flat (per-primitive constant) — the SAME instance's material payload for every pixel
    // of a triangle, mirroring `mat_id`/`mat_albedo` above.
    nointerpolation float4 tex_base_color  : TEXBASECOLOR;
    nointerpolation uint   tex_mat_id      : TEXMATID;
    nointerpolation uint   tex_albedo      : TEXALBEDO;
    nointerpolation uint   tex_normal      : TEXNORMALSLOT;
    nointerpolation uint   tex_metal_rough : TEXMETALROUGH;
    nointerpolation uint   tex_ao          : TEXAO;
    nointerpolation uint   tex_emissive    : TEXEMISSIVE;
    nointerpolation float  tex_metallic    : TEXMETALLIC;
    nointerpolation float  tex_roughness   : TEXROUGHNESS;
#endif
};

// The degeneracy threshold for the instanced normal matrix. Below this |det| the 3x3 inverse
// is numerically unstable (zero-scale collapse, a near-singular mirror), so the instanced arm
// falls back to the M3 `mul(m3, normal)` rather than dividing by ~0.
static const float DET_EPS = 1e-8;

// The cofactor (adjugate / determinant) inverse of a 3x3 matrix. `m` is built row-major from
// the affine rows (`m[i]` is `model.r{i}.xyz`), matching how the instanced arm constructs `m3`.
// The caller guards |det| via DET_EPS before transposing this into the normal matrix, so this
// helper itself does NOT clamp — it returns the raw adjugate/det.
float3x3 inverse3x3(float3x3 m) {
    // Cofactors of each entry (the adjugate is the transpose of the cofactor matrix).
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
    // The adjugate rows are the cofactor COLUMNS; assemble the inverse directly.
    return float3x3(
        c0[0] * inv_det, c1[0] * inv_det, c2[0] * inv_det,
        c0[1] * inv_det, c1[1] * inv_det, c2[1] * inv_det,
        c0[2] * inv_det, c1[2] * inv_det, c2[2] * inv_det);
}

// The determinant of a 3x3 built from the affine's three rows (the degeneracy test feeds the
// W4 guard before any inverse is taken).
float det3x3(float3x3 m) {
    return m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
         - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
         + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
}

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
#ifdef PER_INSTANCE_MATERIAL
        // The PM pipeline never binds an empty (legacy, merged-draw) mesh_draw list
        // (F8 §7a) — this arm's id/albedo are defined-but-unread placeholders, kept
        // only so `mat_id`/`mat_albedo` are initialized on every path.
        output.mat_id = 0u;
        output.mat_albedo = float3(0.0, 0.0, 0.0);
#endif
#ifdef TEXTURED
        // The TEXTURED pipeline never binds an empty (legacy, merged-draw) mesh_draw list
        // either (mirrors the PER_INSTANCE_MATERIAL arm above) — defined-but-unread
        // placeholders, kept only so every TEXTURED varying is initialized on every path.
        output.uv = float2(0.0, 0.0);
        output.world_T = float3(1.0, 0.0, 0.0);
        output.tex_w = 1.0;
        output.tex_base_color = float4(0.0, 0.0, 0.0, 1.0);
        output.tex_mat_id = 0u;
        output.tex_albedo = 0u;
        output.tex_normal = 0u;
        output.tex_metal_rough = 0u;
        output.tex_ao = 0u;
        output.tex_emissive = 0u;
        output.tex_metallic = 0.0;
        output.tex_roughness = 0.5;
#endif
#ifdef MOTION_VECTORS
        // The merged-draw arm has no per-instance model, so this geometry is static in world
        // space (`input.position` IS the world position — `mul(view_proj, p)` above). Motion
        // is camera-only: prev_world == cur_world, so `Δuv` comes purely from the view-proj
        // delta. A still camera ⇒ mc_prev == mc_cur ⇒ cur_clip == prev_clip ⇒ Δuv == 0.
        output.cur_clip  = mul(mc_cur_view_proj,  float4(input.position, 1.0));
        output.prev_clip = mul(mc_prev_view_proj, float4(input.position, 1.0));
#endif
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
        // M4 — correct normal under NON-UNIFORM scale via the inverse-transpose matrix. Under a
        // non-uniform `m3`, `mul(m3, normal)` skews the normal off the surface (it scales the
        // normal like a tangent), so the lit shading on stretched/squashed faces is wrong;
        // `transpose(inverse3x3(m3)) * normal` stays perpendicular to the transformed surface.
        float det = det3x3(m3);
        // W4 degeneracy guard: an unguarded 3x3 inverse on a degenerate transform (zero-scale
        // collapse, a near-singular mirror) divides by ~0 → NaN/Inf normals → black/garbage
        // lighting in the normal MRT. Below DET_EPS, fall back to the M3 `mul(m3, normal)`,
        // which is finite (and as correct as anything is when the basis has collapsed). The
        // branch is a per-vertex `if`; it is wave-coherent for any single uniform-det instance.
        if (abs(det) < DET_EPS) {
            output.normal = mul(m3, input.normal);
        } else {
            float3x3 nm = transpose(inverse3x3(m3));
            output.normal = mul(nm, input.normal);
        }
#ifdef PER_INSTANCE_MATERIAL
        // Indexed IDENTICALLY to `instances[pc.base_instance + instance_id]` above (the
        // SAME expression, M3-proven) — so `mat_id`/`mat_albedo` always name the SAME
        // instance the model matrix does (F8 §7e).
        PerInstanceMaterial pm = instance_materials[pc.base_instance + instance_id];
        output.mat_id = pm.id;
        output.mat_albedo = pm.base_color.xyz;
#endif
#ifdef TEXTURED
        // T6c plan Decision D3: `world_T = normalize(mul(m3, tangent.xyz))` — the PLAIN
        // model 3x3 (`m3`, reused from the position/eye_rel transform above), NOT the
        // inverse-transpose normal matrix `nm` the M4 branch above may compute (a tangent is
        // a SURFACE VECTOR, not a normal — it transforms with the plain Jacobian, unlike a
        // normal, which needs the inverse-transpose under non-uniform scale). `uv`/`tex_w`
        // pass through verbatim; the fragment renormalizes `world_T` after interpolation
        // (Gram-Schmidt) so a non-unit or skewed interpolated tangent is corrected there.
        output.uv = input.uv;
        output.world_T = normalize(mul(m3, input.tangent.xyz));
        // Review O2 (out of campaign scope — no mirrored instances today): the
        // fragment's `B = cross(N, T) * w` handedness is correct only for a
        // POSITIVE-determinant `m3` (a proper rotation/scale); a MIRRORED instance
        // (negative-determinant scale, e.g. one negative axis) flips chirality and would
        // need `w *= sign(det(m3))` here (the SAME `det` the M4 branch above already
        // computes) to keep the bitangent — and thus the sampled normal map — correctly
        // handed.
        output.tex_w = input.tangent.w;
        // Indexed IDENTICALLY to `instances[pc.base_instance + instance_id]` above (the
        // SAME expression, M3-proven) — so the TEXTURED payload always names the SAME
        // instance the model matrix does.
        PerInstanceMaterialTex pmt = instance_materials_tex[pc.base_instance + instance_id];
        output.tex_base_color = pmt.base_color;
        output.tex_mat_id = pmt.material_id;
        output.tex_albedo = pmt.albedo;
        output.tex_normal = pmt.normal;
        output.tex_metal_rough = pmt.metal_rough;
        output.tex_ao = pmt.ao;
        output.tex_emissive = pmt.emissive;
        output.tex_metallic = pmt.metallic;
        output.tex_roughness = pmt.roughness;
#endif
#ifdef MOTION_VECTORS
        // Per-object motion: read the SAME instance slot from the PREVIOUS-frame model ring
        // and place `input.position` in last frame's world space. `world` (above) is this
        // frame's world position. This is what recovers the moving box's TRUE motion vector
        // — its shadow-vis history is then sampled from where the box WAS (the owner's #1
        // in-motion sensitivity). A box at rest has prev model == cur model ⇒ Δuv == 0.
        InstanceModelCol pmodel = prev_instances[pc.base_instance + instance_id];
        float3x3 pm3 = float3x3(pmodel.r0.xyz, pmodel.r1.xyz, pmodel.r2.xyz);
        float3 pt = float3(pmodel.r0.w, pmodel.r1.w, pmodel.r2.w);
        float3 prev_world = mul(pm3, input.position) + pt;
        output.cur_clip  = mul(mc_cur_view_proj,  float4(world, 1.0));
        output.prev_clip = mul(mc_prev_view_proj, float4(prev_world, 1.0));
#endif
    }
    return output;
}
