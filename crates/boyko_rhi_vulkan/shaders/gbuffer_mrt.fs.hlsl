// Render P5-r0 mesh-MRT G-buffer PRODUCER fragment shader.
//
// Writes the mesh fragment's 3 G-buffer attribute lanes in the MARCHER'S EXACT
// encoding (the contract is the marcher/resolve layout, NOT a raster-local one), with
// `mask = 1` so the deferred resolve (`deferred_pbr.hlsl`) lights mesh pixels
// first-class (full Cook-Torrance, identical to an SDF pixel). After r1's ownership
// gate the marcher YIELDS mesh-owned pixels and this fragment stands.
//
//   SV_Target0 (gAlbedo,   R8G8B8A8_UNORM): float4(saturate(base), 1) — RAW LINEAR base.
//   SV_Target1 (gNormal,   R8G8B8A8_UNORM): (oct.x, oct.y, id_ba.x, id_ba.y) — the
//     octahedral world normal in RG + the 16-bit material id in BA.
//   SV_Target2 (gMaterial, R8G8B8A8_UNORM): float4(1, 1, 1, 1) — shadow = ao = 1,
//     mask = 1 (SDF-lit selector ON), alpha 1.
//
// OQ-r0-B: `base` = interpolated LINEAR vertex color; the mesh material id is the default
// `0` (the default material). No material-table fetch / texturing (a charted follow-up).
// The vertex color MUST be linear (the gAlbedo contract is RAW LINEAR base color).
//
// DEVIATION from OQ-r0-B's "material id = a push-constant": the id is a compile-time
// constant `DEFAULT_MESH_MATERIAL_ID = 0` here, NOT a fragment push. A fragment push would
// require broadening the RHI graphics pipeline's VERTEX-only push-constant range to the
// FRAGMENT stage (a cross-cutting change to the shared graphics pipeline builder that
// every graphics pipeline — triangle/present/prepass — would inherit, beyond P5's stated
// edit surface and not validatable on this no-validation-layer box). The OUTPUT is
// byte-identical to a push of the default id 0, so r0's 0%-gate + the mesh-pixel golden
// are unaffected; wiring a real per-mesh id push lands WITH the material-table follow-up
// that actually needs a non-zero id.
//
// The `oct_encode` + `pack_material_id_ba` BODIES are SINGLE-SOURCED in
// `boyko_shaderdsl` and SPLICED here between the `// === GENERATED ... BEGIN/END ===`
// sentinels — the SAME emission the marcher consumes (one source, two splice sites).
// `gbuffer_mrt_edsl_sync.rs` guards the splice (a drift fails CI). NEVER hand-edit the
// generated bodies; re-run `cargo run -p boyko_shaderdsl --features emit --bin
// emit_field` and re-splice. The signatures + framing + raster I/O are hand-written.
//
// Rung-3b MOTION_VECTORS variant (opt-in, compiled with `-D MOTION_VECTORS=1`): adds a 4th
// MRT `SV_Target3 motion_vec` (R16G16_SFLOAT) carrying `Δuv = clip_to_uv(prev_clip) -
// clip_to_uv(cur_clip)` — the per-object + camera motion vector the temporal shadow-vis
// reprojection samples the history with. Both clip positions arrive as VS varyings and are
// divided here through the IDENTICAL `clip_to_uv`, so a static pixel writes exactly `(0,0)`.
// All new I/O is gated under `#ifdef MOTION_VECTORS`, so the base compile is byte-frozen
// (the `gbuffer_mrt.fs.spv` golden is untouched — the Rung-3b step-5 byte-identity gate).
//
// Asset-streaming plan F8 PER_INSTANCE_MATERIAL variant (opt-in, compiled with
// `-D PER_INSTANCE_MATERIAL=1`): reads the flat per-instance `mat_id` varying the VS
// forwards and packs IT (instead of the compile-time `DEFAULT_MESH_MATERIAL_ID`) into
// `gNormal.BA`. The WHOLE `id_ba` statement is under `#ifdef/#else/#endif`, with the
// `#else` arm CHARACTER-FOR-CHARACTER the base line, so the base (no-`-D`) compile
// emits the identical token stream there — the `gbuffer_mrt.fs.spv` golden is untouched
// (mirrors the MOTION_VECTORS discipline above; the two variants are mutually
// exclusive — never compiled together this rung).
//
// F8+ (owner: material-drives-albedo-too): the SAME variant ALSO sources `gAlbedo`
// from the flat per-instance `mat_albedo` varying (the material's LINEAR base_color)
// instead of the interpolated vertex color, so a material genuinely controls what the
// mesh looks like, not just its metallic/roughness (`mrr`). The WHOLE `albedo`
// statement is under `#ifdef/#else/#endif` the same way, `#else` CHARACTER-FOR-
// CHARACTER the pre-F8+ line — the base compile is untouched by construction.
//
// Textured-PBR rung T6c TEXTURED variant (opt-in, compiled with `-D TEXTURED=1`): an
// INDEPENDENT #ifdef axis from PER_INSTANCE_MATERIAL/MOTION_VECTORS (T6c plan Decision D4 /
// locked decision 1 — never compiled together with either). Samples the bindless texture
// array (set 1, binding 0 — a runtime-sized `Texture2D[]`, `NonUniformResourceIndex`-gated,
// the SPIR-V descriptor-indexing proof this rung's hermetic test checks) through the shared
// immutable sampler (set 1, binding 1):
//   * gAlbedo sources the sampled+modulated albedo texture (falls back to `base_color` when
//     the slot is `0` — the reserved T4 error-texture slot, never a real material's texture).
//   * gNormal's world normal is TANGENT-SPACE normal-mapped when the normal-map slot is
//     bound, else stays the geometric vertex normal. The TBN BASIS follows glTF/Mikktspace
//     convention (`w` on the bitangent sign); the sampled GREEN channel is SEPARATELY
//     negated per this engine's own convention for OpenGL-style input maps — see the FS
//     `main`'s GREEN-CHANNEL CONVENTION block (not glTF/Mikktspace) for why.
//   * A 4th MRT `SV_Target3 pbr` (`R16G16B16A16_SFLOAT`) carries `[metallic, roughness,
//     AO-modulation, emissive-luminance-modulation]` — sampled from the metal-rough/AO/
//     emissive textures when their slots are bound, else the material's scalar fallback
//     (`tex_metallic`/`tex_roughness` for the first two; `1.0` for AO/emissive). The
//     deferred SOFTWARE resolve (`deferred_pbr.hlsl`, T6a) reads this UNCONDITIONALLY when
//     `MaterialGpu.mrr[3]`'s TEXTURED bit is set; the HARDWARE resolve variants have no
//     `gPbr` binding (T6a plan Decision D1 — software-resolve-only), so under a
//     hardware-ROUTED resolve the metallic/roughness/AO/emissive silently fall back to the
//     material's scalar `mrr` even though gAlbedo/gNormal were still sampled.
// EVERYTHING new is gated under `#ifdef TEXTURED`, so the base (no-`-D`) compile is
// byte-frozen — the `gbuffer_mrt.fs.spv` golden is untouched (every existing `#ifdef
// PER_INSTANCE_MATERIAL ... #else ... #endif` site is WRAPPED, unmodified, inside a new
// outer `#ifdef TEXTURED ... #else ... #endif`, so neither its PM nor its base arm's
// characters change).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 gbuffer_mrt.fs.hlsl -Fo gbuffer_mrt.fs.spv
//   (MOTION_VECTORS variant: add `-D MOTION_VECTORS=1 -Fo gbuffer_mrt_mv.fs.spv`)
//   (PER_INSTANCE_MATERIAL variant: add `-D PER_INSTANCE_MATERIAL=1 -Fo gbuffer_mrt_pm.fs.spv`)
//   (TEXTURED variant: add `-D TEXTURED=1 -Fo gbuffer_mrt_tex.fs.spv`)

// OQ-r0-B: the mesh's 16-bit material id. The default material (id 0); see the DEVIATION
// note in the header for why this is a constant, not a fragment push, in r0.
static const uint DEFAULT_MESH_MATERIAL_ID = 0u;

// The PERSPECTIVE mesh-depth normalizer: this fragment writes `md = length(eye_rel) /
// MESH_DEPTH_T_MAX`, and the marcher decodes the SAME pixel as `t_mesh = md *
// MESH_DEPTH_T_MAX` (`sdf_gbuffer_composite.hlsl`, the CAM_PERSPECTIVE arm). The
// normalizer CANCELS in the encode→decode round-trip, so `t_mesh == length(eye_rel)`
// regardless of its value — it only sets the [0,1] depth-buffer RANGE the encode can
// represent. It is DECOUPLED from the marcher's ray-miss bound `T_MAX` (= 10, the SDF
// trace length): raster mesh geometry can stand far past the SDF horizon (a long floor /
// back wall), so a small `T_MAX` would SATURATE the depth to the no-mesh clear (1.0) and
// the marcher would read that far geometry as background → broken CSM/lighting on it (the
// 3-cascade demo's receding floor + far casters). `64` covers any room-scale eye distance
// with float32 headroom. Mirrors `compute::MESH_DEPTH_T_MAX`; the raster shaders `#include`
// nothing, so the literal is duplicated here (the `instanced_vs_host_mirror` sync-pin
// asserts host == this). The ORTHO arm below does NOT use this — it writes the MVP's
// `position.z` (encoded with the marcher `T_MAX` the ortho projection bakes in).
static const float MESH_DEPTH_T_MAX = 64.0;

struct PsIn {
    float4 position : SV_Position;
    float4 color    : COLOR0;
    float3 normal   : NORMAL;
    float3 eye_rel  : WORLDDIST;   // cam_eye.xyz - world position (perspective-correct)
    float  cam_mode : CAMMODE;     // 0 = ortho, 1 = perspective
#ifdef PER_INSTANCE_MATERIAL
    nointerpolation uint mat_id : MATID;
    nointerpolation float3 mat_albedo : MATALBEDO;
#endif
#ifdef MOTION_VECTORS
    float4 cur_clip  : CURCLIP;    // mc_cur_view_proj  * cur_world  (marcher-aligned clip)
    float4 prev_clip : PREVCLIP;   // mc_prev_view_proj * prev_world (marcher-aligned clip)
#endif
#ifdef TEXTURED
    float2 uv       : TEXUV;
    float3 world_T  : TEXTANGENT;
    float  tex_w    : TEXHAND;
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

struct PsOut {
    float4 albedo   : SV_Target0;  // -> gAlbedo
    float4 normal   : SV_Target1;  // -> gNormal
    float4 material : SV_Target2;  // -> gMaterial
#ifdef MOTION_VECTORS
    float2 motion_vec : SV_Target3; // -> motion_vec (R16G16_SFLOAT) Δuv, prev - cur
#endif
#ifdef TEXTURED
    // -> gPbr (R16G16B16A16_SFLOAT): [metallic, roughness, AO-modulation,
    // emissive-luminance-modulation]. MOTION_VECTORS and TEXTURED are never compiled
    // together (T6c plan Decision D4), so both may occupy SV_Target3 without collision.
    float4 pbr : SV_Target3;
#endif
    float  depth    : SV_Depth;    // -> the shared D32 depth the marcher samples as `md`
};

#ifdef TEXTURED
// Textured-PBR rung T6c: the bindless texture-array set (set 1 — DISTINCT from the gbuffer
// producer's set 0). Binding 0 is a runtime-sized `SAMPLED_IMAGE` array
// (`boyko_rhi_vulkan::bindless::BINDLESS_IMAGE_BINDING`); binding 1 is the ONE shared
// immutable trilinear+anisotropic sampler
// (`boyko_rhi_vulkan::bindless::BINDLESS_SAMPLER_BINDING`). Slot `0` is the T4 reserved
// error-texture slot — `register` never issues it, so every real material slot is `!= 0`;
// callers gate each sample with `slot != 0`.
[[vk::binding(0, 1)]] Texture2D gTextures[] : register(t0, space1);
[[vk::binding(1, 1)]] SamplerState gTexSampler : register(s0, space1);
#endif

#ifdef MOTION_VECTORS
// Marcher-aligned clip -> [0,1]^2 screen UV. The projection (`marcher_view_proj_rows`)
// already bakes the y-flip into clip.y (sy = -1/tan), so this is the plain NDC remap with
// NO extra negation: uv = (clip.xy / clip.w) * 0.5 + 0.5. Applied identically to cur_clip
// and prev_clip, so the constant 0.5 offset + scale cancel in the Δuv difference and a
// static pixel yields (0,0). The UV origin is top-left (Vulkan framebuffer convention), so
// the temporal reprojection samples the history at `pixel_uv + Δuv` directly.
float2 clip_to_uv(float4 clip) {
    return (clip.xy / clip.w) * 0.5 + 0.5;
}
#endif

// Octahedral-encode a unit normal `n` into [0,1]^2, the marcher/resolve's exact fold.
// The BODY is eDSL-single-sourced (boyko_shaderdsl::oct::oct_encode_body); the resolve
// decodes it via `oct_decode`. Spliced verbatim from the marcher's identical span.
float2 oct_encode(float3 n) {
    // === GENERATED oct_encode BEGIN === (boyko_shaderdsl::oct::oct_encode_body)
    n = n / (abs(n.x) + abs(n.y) + abs(n.z));
    float2 e = n.xy;
    if (n.z < 0.0) {
        e = (1.0 - abs(e.yx)) * float2(e.x >= 0.0 ? 1.0 : -1.0, e.y >= 0.0 ? 1.0 : -1.0);
    }
    return e * 0.5 + 0.5;
    // === GENERATED oct_encode END ===
}

// Pack a 16-bit material id into the B + A channels of an RGBA8 texel (low byte -> B,
// high byte -> A). The BODY is eDSL-single-sourced
// (boyko_shaderdsl::pack::pack_material_id_ba_body); the resolve reconstructs
// `id = round(b*255) | (round(a*255) << 8)`. Spliced verbatim from the marcher's span.
float2 pack_material_id_ba(uint id) {
    // === GENERATED pack_material_id_ba BEGIN === (boyko_shaderdsl::pack::pack_material_id_ba_body)
    uint lo = id & 255u;
    uint hi = id >> 8u & 255u;
    return float2((float)lo / 255.0, (float)hi / 255.0);
    // === GENERATED pack_material_id_ba END ===
}

PsOut main(PsIn input) {
    PsOut output;
    // gAlbedo: RAW LINEAR base color (saturated to the UNORM range), alpha 1.
    // F8+ (owner: material-drives-albedo-too): the PER_INSTANCE_MATERIAL variant sources
    // it from the per-instance material's `base_color` instead of the mesh vertex color,
    // so a material genuinely controls what the mesh looks like (not just `mrr`). The
    // `#else` arm is CHARACTER-FOR-CHARACTER the pre-F8+ line — the base (no-`-D`)
    // compile is byte-frozen (the `gbuffer_mrt.fs.spv` golden is untouched).
    // T6c plan Decision D4: the ENTIRE pre-T6c `#ifdef PER_INSTANCE_MATERIAL / #else /
    // #endif` block below is WRAPPED, byte-UNMODIFIED, inside a new outer `#ifdef TEXTURED /
    // #else / #endif` — neither its PM nor its base line changes a single character, so both
    // the base and PM compiles stay byte-frozen.
#ifdef TEXTURED
    // gAlbedo: the sampled albedo texture (sRGB view -> hw-linear on sample) modulated by
    // the material's base_color, or base_color alone when no albedo texture is bound (T6c
    // plan Decision 5). Slot `0` is the reserved T4 error-texture slot, never a real
    // material's texture — gated `!= 0`.
    float3 albedo_tex_rgb = float3(1.0, 1.0, 1.0);
    if (input.tex_albedo != 0u) {
        albedo_tex_rgb = gTextures[NonUniformResourceIndex(input.tex_albedo)].Sample(gTexSampler, input.uv).rgb;
    }
    float3 tex_albedo_out = (input.tex_albedo != 0u)
        ? albedo_tex_rgb * input.tex_base_color.rgb
        : input.tex_base_color.rgb;
    output.albedo = float4(saturate(tex_albedo_out), 1.0);
#else
#ifdef PER_INSTANCE_MATERIAL
    output.albedo = float4(saturate(input.mat_albedo), 1.0);
#else
    output.albedo = float4(saturate(input.color.rgb), 1.0);
#endif
#endif
    // gNormal: the octahedral world normal in RG + the packed 16-bit material id in BA.
    float3 n = normalize(input.normal);
#ifdef TEXTURED
    // Tangent-space normal mapping (T6c plan Decision 3): renormalize the interpolated
    // geometric normal FIRST, Gram-Schmidt the interpolated tangent against it, derive the
    // bitangent via the glTF/Mikktspace handedness sign (`w` multiplies the BITANGENT —
    // matches `boyko_render::tangent`'s Lengyel `w` convention; THIS part of the basis IS
    // glTF/Mikktspace), sample + unpack + renormalize the tangent-space normal (trilinear
    // mip sampling denormalizes), then rotate it into world space via the TBN basis.
    // `normal_slot == 0` keeps the geometric normal unperturbed. The sampled GREEN channel
    // is separately negated below — see the GREEN-CHANNEL CONVENTION block: that negation
    // is THIS ENGINE's own convention for OpenGL-style input maps, NOT glTF/Mikktspace.
    //
    // GREEN-CHANNEL CONVENTION (settled by the numeric real-bake oracle + the synthetic
    // bump/marker ground-truth renders, 2026-07-12): the engine's native normal-map input
    // convention is OpenGL-style (+G = a slope facing image-UP — the dominant third-party
    // PBR convention), and under THIS engine's Lengyel bake the sampled green must be
    // NEGATED. Why: on a v-down-parameterized mesh the real bake yields `w = +1` with
    // `B = cross(N, T) * w` pointing image-DOWN (verified numerically on the actual
    // `generate_tangents` output — NOT hand-derived), so a raw `+G` tilt would push the
    // normal image-DOWN and a known-protruding OGL bump grid renders as vertically-inverted
    // dents (the synthetic-marker render). Bevy renders the same file correctly WITHOUT a
    // flip because its mikktspace tangents carry the OPPOSITE handedness on such meshes —
    // the flip below reproduces the identical, physically-correct response in this basis.
    // Brick-like content cannot adjudicate this convention (bump/dent ambiguity); only a
    // known-geometry map can. DirectX-style (+G down) maps must be pre-flipped at pack time.
    if (input.tex_normal != 0u) {
        float3 N = n;
        float3 T = normalize(input.world_T - dot(input.world_T, N) * N);
        float3 B = cross(N, T) * input.tex_w;
        float3 packed_n = gTextures[NonUniformResourceIndex(input.tex_normal)].Sample(gTexSampler, input.uv).xyz;
        float3 n_ts = normalize(packed_n * 2.0 - 1.0);
        n_ts.y = -n_ts.y;
        n = normalize(T * n_ts.x + B * n_ts.y + N * n_ts.z);
    }
#endif
    float2 oct = oct_encode(n);
    // Asset-streaming plan F8: the PER_INSTANCE_MATERIAL variant packs the REAL
    // per-instance id (already CPU OOB-clamped, F8 §4.2); the base compile keeps the
    // compile-time default (the `#else` arm is CHARACTER-FOR-CHARACTER the pre-F8 line —
    // the frozen-base guarantee by construction, F8 §3.2). T6c: the SAME wrap discipline as
    // gAlbedo above — the inner block is byte-UNMODIFIED.
#ifdef TEXTURED
    float2 id_ba = pack_material_id_ba(input.tex_mat_id);
#else
#ifdef PER_INSTANCE_MATERIAL
    float2 id_ba = pack_material_id_ba(input.mat_id);
#else
    float2 id_ba = pack_material_id_ba(DEFAULT_MESH_MATERIAL_ID);
#endif
#endif
    output.normal = float4(oct.x, oct.y, id_ba.x, id_ba.y);
    // gMaterial: shadow = 1, ao = 1, mask = 1 (SDF-lit -> Cook-Torrance in the resolve).
    // Analytic mesh shadow/AO via the SDF march is a charted follow-up, NOT P5.
    output.material = float4(1.0, 1.0, 1.0, 1.0);
#ifdef TEXTURED
    // gPbr (T6c plan Decision 5): [metallic, roughness, AO-modulation,
    // emissive-luminance-modulation]. glTF channel convention for the packed metal-rough
    // texture: metallic = B, roughness = G. AO/emissive default to `1.0` (no
    // occlusion / no luminance mask) when their slots are unbound.
    float pbr_metallic = input.tex_metallic;
    float pbr_roughness = input.tex_roughness;
    if (input.tex_metal_rough != 0u) {
        float3 mr = gTextures[NonUniformResourceIndex(input.tex_metal_rough)].Sample(gTexSampler, input.uv).rgb;
        pbr_metallic = mr.b;
        pbr_roughness = mr.g;
    }
    float pbr_ao = 1.0;
    if (input.tex_ao != 0u) {
        pbr_ao = gTextures[NonUniformResourceIndex(input.tex_ao)].Sample(gTexSampler, input.uv).r;
    }
    float pbr_emissive = 1.0;
    if (input.tex_emissive != 0u) {
        float3 em = gTextures[NonUniformResourceIndex(input.tex_emissive)].Sample(gTexSampler, input.uv).rgb;
        pbr_emissive = dot(em, float3(0.2126, 0.7152, 0.0722));
    }
    output.pbr = float4(pbr_metallic, pbr_roughness, pbr_ao, pbr_emissive);
#endif
    // SV_Depth: the shared depth the marcher samples as `md`.
    //   * PERSPECTIVE (cam_mode == 1): `rd` is UNIT, so the marcher's `P = ro + rd*t_mesh`
    //     wants `t_mesh` = the EUCLIDEAN eye->surface distance => md = length(eye_rel) /
    //     MESH_DEPTH_T_MAX (decoded `t_mesh = md * MESH_DEPTH_T_MAX`, the normalizer cancels).
    //   * ORTHO (cam_mode == 0): the marcher's per-pixel `ro.xy == P.xy` with `rd = (0,0,-1)`,
    //     so `t_mesh` is the AXIAL `CAM_Z - z`, which the MVP already encodes into the
    //     rasterized SV_Position.z (= `(CAM_Z - z)/T_MAX`, the marcher `T_MAX`). Writing it
    //     back unchanged is byte-identical to NOT writing SV_Depth — the 41 ortho goldens
    //     are preserved (the composite decodes the ortho arm with the marcher `T_MAX`).
    output.depth = (input.cam_mode > 0.5) ? (length(input.eye_rel) / MESH_DEPTH_T_MAX) : input.position.z;
#ifdef MOTION_VECTORS
    // 4th MRT: Δuv = where-this-surface-was minus where-it-is, in [0,1] screen UV. The
    // temporal reprojection reads `hist` at `pixel_uv + motion_vec`. Static ⇒ (0,0).
    output.motion_vec = clip_to_uv(input.prev_clip) - clip_to_uv(input.cur_clip);
#endif
    return output;
}
