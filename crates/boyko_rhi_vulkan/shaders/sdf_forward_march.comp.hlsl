// Multi-paradigm render-path plan, rung R-SDFFWD (SDF forward-march wiring): the FUSED
// march-then-shade compute pass for the SDF geometry leg under a non-Deferred render path
// (`Forward`/`ForwardPlus`). Unlike the deferred G-buffer marcher (`sdf_gbuffer_composite.hlsl`,
// which WRITES ATTRIBUTES for a later fullscreen resolve), this pass has no resolve to hand off
// to — it marches the SDF field AND runs the full Cook-Torrance shade in ONE dispatch, storing
// the lit color straight into the Forward `lit` image (the SAME target `forward_opaque.fs.hlsl`
// writes via a raster COLOR attachment; this pass writes it via a STORAGE image, C5's
// COLOR_ATTACHMENT -> GENERAL -> SHADER_READ chain).
//
// # Verbatim-copied spans (pinned by `sdf_field_edsl_sync.rs`)
//
// The field-eval/ray-gen machinery is NOT copied — it lives in the shared
// `sdf_field.hlsli`/`ray_gen.hlsli` headers, `#include`d exactly as
// `sdf_gbuffer_composite.hlsl` does (the determinism-frozen field gateway). The BRICK/CLIP-MAP
// acceleration (M1 empty-skip + M2 trilinear/JCGT-cubic SURFACE bricks + M4 clip-map LOD
// selection) and the A1 analytic soft-shadow march physically live IN
// `sdf_gbuffer_composite.hlsl` itself, so those spans are copied VERBATIM (byte-for-byte,
// including every `=== GENERATED ... BEGIN/END ===` sentinel) from that file:
// `dist_to_brick_exit`, `brick_cell_class`, `m2_decode`, `m2_corner`, `m2_clamp_index`,
// `m2_cubic_eval`, `m2_jcgt_cubic_coeffs`, `m2_regula_falsi`, `m2_marmitt_root`, `m2_brick_span`,
// `m2_brick_cubic_hit`, `m2_surface_hit` (+ its `m2_surface_hit_refine` GENERATED span),
// `select_level`, and `sdf_soft_shadow` (+ its GENERATED penumbra span). Every GENERATED span is
// pinned by a `sdf_field_edsl_sync.rs` test extended THIS rung to assert `.contains()` against
// THIS file too (in addition to `sdf_gbuffer_composite.hlsl`) — a hand-edit of either committed
// copy fails CI. `MeshSdf` (MDF Stage-2c, the dedicated mesh-SDF shadow caster) and Render B1
// (Keinert over-relaxation) are NOT copied — out of this rung's scope (B1 is a pure speed
// optimization over the plain sphere-trace this pass uses; MDF is a separate shadow-caster
// feature). The M1/M2/M4 acceleration is wired to REAL, live (shared) resources but threaded
// OFF (`brick_enabled`/`brick_trilinear` = 0, `brick_levels` = 0) at this rung's host call site —
// mirroring the EXACT 0%-gate precedent M1/M2/M4 themselves used when they first landed in the
// deferred marcher (see this file's push-constant doc below).
//
// # Shading (cloned from `forward_opaque.fs.hlsl`, token-for-token where noted)
//
// After a hit, `Surface` is built from the march (gradient normal `sdf_normal`, material via the
// SAME nearest-edit argmin attribution the deferred marcher's `pick_material_id` uses), then the
// ALL-LIGHTS direct loop + `eval_pbr_ambient_hemi`/`eval_pbr_sun_disc` + the tonemap tail are a
// TOKEN-FOR-TOKEN clone of `forward_opaque.fs.hlsl`'s own loop (CSM/atlas visibility +
// attenuation shapes unchanged) — see that file's own "Duplicated (not shared) pure helpers"
// doc for why `env_brdf_approx`/`sun_kernel*`/`SUN_ENV_WEIGHT` are duplicated here too, not
// factored into `pbr_lighting.hlsli`. The ONE shading difference from `forward_opaque.fs.hlsl`:
// the PRIMARY directional's visibility starts at THIS pass's own `sdf_soft_shadow` analytic
// march (the SDF's baked self-shadow) instead of the mesh leg's implicit `1.0`, then CSM
// min-combines in exactly as the deferred resolve's `is_sdf_lit` branch does for the SAME
// primary-directional case (`deferred_pbr.hlsl`: `vis = shadow; ... vis = min(vis,
// csm_visibility(...))` — `shadow` there is the marcher's pre-baked `gMaterial.r`; here it is
// computed in-register since march and shade are fused). No SDF-side AO term is applied
// (`ao_final` stays the Forward v1 mesh-leg constant `1.0`, mirroring
// `forward_opaque.fs.hlsl`'s own "no baked AO term, no SSAO consumer" v1 scope cut — see this
// rung's report for the rationale).
//
// # HAS_MESH ownership gate (Decision 4's consumer half) and the VIEWT producer matrix
//
// Compiled FOUR times from this ONE source — the `{HAS_MESH} x {VIEWT}` variant matrix
// (mirrors `forward_opaque.fs.hlsl`'s FROXEL idiom; the two no-`VIEWT` compiles are the
// original pair, byte-identical across this rung):
//   * `-D HAS_MESH=1` -> `sdf_forward_march.comp.spv`: the raster mesh leg is present. Samples
//     the Forward reverse-Z `forward_depth` image, inverts it to a view-space `view_z_mesh` via
//     `boyko_render::view::forward_view_z_from_depth`'s EXACT algebraic inverse (`view_z = B /
//     (depth - A)`, `A`/`B` precomputed host-side from `near`/`far` and pushed), converts to the
//     SAME ray-parameter metric the march uses (`t_mesh = view_z_mesh / dot(cam_forward, rd)` —
//     sound because `ro == cam_eye` for a perspective ray, `generate_ray`'s own contract), and
//     bounds the march at `t_mesh` (mirrors the deferred marcher's own `if (t >= t_mesh) break;`
//     mesh-occlusion guard). `sdf_owns = hit && t < t_mesh`.
//   * (no `-D`) -> `sdf_forward_march_sdfonly.comp.spv`: no mesh leg (`GeometryLegs::Sdf`) — no
//     `forward_depth` binding is declared (the layout still declares it, per the R2
//     bound-but-unread contract every M2/M4/MDF binding already establishes in
//     `sdf_gbuffer_composite.hlsl`; this variant's SPIR-V just never references it) and
//     `sdf_owns = hit` unconditionally.
// A miss (`!sdf_owns`) writes NOTHING to `gLit` — the sky/mesh color `forward_opaque.fs.hlsl`
// already painted this frame stands (the SAME "misses write nothing" contract the deferred
// marcher's own G-buffer ownership gate documents).
//   * `-D VIEWT=1` (x either mesh define) -> `sdf_forward_march_viewt.comp.spv` /
//     `sdf_forward_march_sdfonly_viewt.comp.spv`: the TAA-under-VB gViewT PRODUCER variants.
//     On a TAA-armed SDF-carrying VisibilityBuffer leg (`VB x Both` / `VB x Sdf`) this marcher
//     IS the composite and therefore the SOLE `gViewT` producer — the standalone
//     `viewt_from_depth_rz.comp.hlsl` pass covers only the `VB x Mesh` (marcher-less) config,
//     exactly as `viewt_from_depth.comp.hlsl` covers only `Deferred x Mesh` while
//     `sdf_gbuffer_composite.hlsl` owns the lane on every SDF-carrying Deferred leg. The gViewT
//     write discipline is that deferred marcher's own (its u8 precedent): EVERY in-bounds pixel
//     written EXACTLY ONCE — SDF-owned pixels store the marched ray parameter `t`, mesh-owned
//     pixels the decoded `t_mesh`, background the `1.0e30` sentinel. The no-`VIEWT` variants
//     compile the write out entirely (their SPIR-V never references binding 13), so the
//     TAA-off render paths keep a zero-cost marcher — the 0%-gate.
//
// # Bindings (Set 0, own dedicated vocabulary — NOT the deferred marcher's `vocab_layout`: this
// # pass writes `gLit` directly, needs no G-buffer attribute images, and shares the light
// # table/material table Forward's own Set 0 already carries)
//
//   t0  : StructuredBuffer<uint>              Buf           edit-list header (READ-ONLY)
//   t1  : StructuredBuffer<uint>               LightBuf      Lighting L0 light table
//   t2  : StructuredBuffer<MaterialGpu>        Materials     PBR material table
//   b3  : cbuffer Camera                                     80-byte extent/camera block (SAME
//                                                             shape as `forward_opaque.fs.hlsl`'s
//                                                             Camera @2 / the marcher's b5 head)
//   u4  : RWTexture2D<float4> (rgba8)          gLit          Forward's `lit` STORAGE image
//   t5  : StructuredBuffer<uint>               PointerGrid   M1/M4 level-0 empty-skip grid
//   t6/s6: Texture3D<float>+SamplerState       BrickAtlas    M2/M4 level-0 trilinear atlas
//   t7  : StructuredBuffer<uint>               PointerGrid1  M4 level-1 empty-skip grid
//   t8/s8: Texture3D<float>+SamplerState       BrickAtlas1   M4 level-1 trilinear atlas
//   t9  : StructuredBuffer<uint>               PointerGrid2  M4 level-2 empty-skip grid
//   t10/s10: Texture3D<float>+SamplerState     BrickAtlas2   M4 level-2 trilinear atlas
//   b11 : cbuffer BrickLevels                                `M4Level m2_levels[BRICK_LEVELS]`
//                                                             (don't-care while `brick_levels==0`)
//   t12 : Texture2D<float>  gForwardDepth  (HAS_MESH-declared only) Forward reverse-Z depth
//   u13 : RWTexture2D<float> (r32f)         gViewT        (VIEWT-declared only) the TAA
//                                                          ray-parameter lane (`core.viewt`)
//
// Set 1 (shadow) is a VERBATIM copy of `forward_opaque.fs.hlsl`'s own Set-1 block (CSM cascades +
// punctual atlas), so the SAME physical descriptor set (`ForwardTargets::set1`) binds to BOTH
// pipelines — `shadow_apply.hlsli`'s INCLUDE CONTRACT (fixed global names, any binding numbers)
// is satisfied identically.
//
// # Push constant (own dedicated 40-byte contract — a NEW pass, not sharing
// # `FineMarcherPush`/`GBUFFER_MARCHER_PUSH_BYTES`)
//
//   offset  0 : uint   extent_w        render extent width (the dispatch bounds `idx < w*h`)
//   offset  4 : uint   extent_h        render extent height
//   offset  8 : float  view_z_a        HAS_MESH reverse-Z decode `A` (don't-care w/o HAS_MESH)
//   offset 12 : float  view_z_b        HAS_MESH reverse-Z decode `B`
//   offset 16 : float3 light_dir       primary directional light direction (un-normalized)
//   offset 28 : uint   brick_enabled   M1 empty-skip gate; 0 = OFF (byte-identical-analytic)
//   offset 32 : uint   brick_trilinear M2 trilinear+cubic gate; 0 = OFF
//   offset 36 : uint   brick_levels    M4 clip-map level count; 0 = OFF (no level ever selected)
// 40 bytes total. The host (`GpuSceneBundles::boot`, this rung) always threads
// `brick_enabled = brick_trilinear = brick_levels = 0` — the acceleration CODE is genuinely live
// in the compiled SPIR-V (a runtime push read, never constant-folded), bound against REAL shared
// resources (the SAME `PointerGrid`/`BrickAtlas`/... buffers+textures the deferred marcher
// binds), but dynamically inactive this rung — activating it on the Forward leg is a follow-up
// (mirrors how M1/M2/M4 themselves landed OFF-by-default in the deferred marcher first, then a
// later rung flipped the host gate).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) FOUR times from this ONE
// source (the `{HAS_MESH} x {VIEWT}` matrix):
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main -D HAS_MESH=1 \
//       -fspv-target-env=vulkan1.3 sdf_forward_march.comp.hlsl -Fo sdf_forward_march.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_forward_march.comp.hlsl -Fo sdf_forward_march_sdfonly.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main -D HAS_MESH=1 -D VIEWT=1 \
//       -fspv-target-env=vulkan1.3 sdf_forward_march.comp.hlsl -Fo sdf_forward_march_viewt.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main -D VIEWT=1 \
//       -fspv-target-env=vulkan1.3 sdf_forward_march.comp.hlsl -Fo sdf_forward_march_sdfonly_viewt.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe sdf_forward_march.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe sdf_forward_march_sdfonly.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe sdf_forward_march_viewt.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe sdf_forward_march_sdfonly_viewt.comp.spv

StructuredBuffer<uint> Buf : register(t0); // binding 0: edit-list header (READ-ONLY)

// The shared determinism-frozen field gateway (`Buf` above must be in scope first — the same
// INCLUDE CONTRACT `sdf_gbuffer_composite.hlsl` documents). `sdf`/`sdf_normal`/`field_distance`
// stay byte-identical to every other consumer of this header.
#include "sdf_field.hlsli"

// Shared camera ray-generation (the SAME header the marcher + the deferred resolve share).
#include "ray_gen.hlsli"

// binding 1: the Lighting-L0 light table (word-indexed `[LightHeaderGpu || GpuLight[]]`) — the
// SAME SSBO Forward's own Set 0 (`forward_opaque.fs.hlsl`) and the deferred resolve read.
[[vk::binding(1, 0)]] StructuredBuffer<uint> LightBuf;

// binding 2: the material table — byte-identical `MaterialGpu` shape to every other consumer
// (`base_color`/`mrr`/`emissive`).
struct MaterialGpu {
    float4 base_color;
    float4 mrr;
    float4 emissive;
};
[[vk::binding(2, 0)]] StructuredBuffer<MaterialGpu> Materials;

// Camera modes selected by `cam.camera_mode` (mirrors `sdf_gbuffer_composite.hlsl`'s own
// aliases). Forward is perspective-only in practice (`forward_view_proj_rows`'s invariant), but
// the shared `generate_ray` handles both.
static const uint CAM_ORTHO       = RAYGEN_CAM_ORTHO;
static const uint CAM_PERSPECTIVE = RAYGEN_CAM_PERSPECTIVE;

// binding 3: the extent/camera UNIFORM block — the SAME 80-byte shape
// `forward_opaque.fs.hlsl`'s own Camera @2 (and the marcher's b5 head) declare.
[[vk::binding(3, 0)]] cbuffer Camera {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};

// binding 4: Forward's `lit` STORAGE image — the SAME physical image
// `forward_opaque.fs.hlsl` writes via a raster COLOR attachment (C5: two per-path producer
// accesses on one image, boot-mutually-exclusive paths never contend). `[[vk::image_format]]`
// pins the `OpTypeImage` format (`shaderStorageImageWriteWithoutFormat` is OFF at device
// creation — the SAME M2/MRT precedent every other `RWTexture2D` in this shader family follows).
[[vk::image_format("rgba8")]] RWTexture2D<float4> gLit : register(u4);

// --- SDF brick-atlas M1/M2/M4 acceleration (VERBATIM copy from `sdf_gbuffer_composite.hlsl` —
// see this file's header doc for the "wired to real resources, threaded OFF this rung" note) ---

// M2 brick geometry (mirror `boyko_sdf_math::brick` + `boyko_rhi_vulkan::compute`).
static const uint  M2_BRICK_INTERIOR = 8u;     // BRICK_INTERIOR
static const uint  M2_BRICK_ALLOC    = 10u;    // BRICK_ALLOC (interior + 1-voxel apron each face)
static const uint  M2_GRID_DIM       = 4u;     // M2_GRID_DIM — per-axis cell count (M5 toroidal mask DIM)
static const float M2_APRON          = 1.0;    // APRON (one voxel)
static const float M2_ATLAS_BIAS     = 0.0;    // ATLAS_SAMPLE_BIAS (golden-locked to 0)
static const float M2_CUBIC_ROOT_EPS = 1.0e-6; // CUBIC_ROOT_EPS (root residual / bracket tol)
static const uint  M2_MARMITT_ITERS  = 8u;     // MARMITT_ITERS (regula-falsi cap)
static const uint  M2_MAX_CELLS      = 30u;    // 3 * BRICK_ALLOC — the longest 3D-DDA path
static const float M2_CREASE_EPS     = 0.0192; // analytic-residual crease band (world units)
static const uint  M2_REFINE_ITERS   = 32u;    // signed accept-refine iteration cap
static const float M2_REFINE_RELAX   = 0.8;    // signed accept-refine under-relaxation factor

// One clip-map level's b5-style UBO block (mirror `sdf_gbuffer_composite.hlsl`'s `M4Level`).
struct M4Level {
    float4 origin_brick_world;   // xyz = level grid origin, w = brick_world_at_level(L)
    float4 dims_atlas_dim;       // xyz = grid dims (f32), w = atlas_dim (f32); read via (uint) cast
    float4 band_voxel_inv_atlas; // x = band_half(L), y = voxel_size(L), z = 1/atlas_dim, w = level L
};
static const uint BRICK_LEVELS = 3u;

// binding 11: the N-level clip-map geometry UBO (don't-care while `pc.brick_levels == 0`).
[[vk::binding(11, 0)]] cbuffer BrickLevels {
    M4Level m2_levels[BRICK_LEVELS];
};

// binding 5: M1/M4 level-0 pointer grid.
StructuredBuffer<uint> PointerGrid : register(t5);
// binding 6: M2/M4 level-0 trilinear atlas (combined image+sampler).
[[vk::binding(6, 0)]] Texture3D<float>  BrickAtlas   : register(t6);
[[vk::binding(6, 0)]] SamplerState      BrickSampler : register(s6);
// binding 7/9: M4 level-1/level-2 pointer grids.
StructuredBuffer<uint> PointerGrid1 : register(t7);
StructuredBuffer<uint> PointerGrid2 : register(t9);
// binding 8/10: M4 level-1/level-2 trilinear atlases.
[[vk::binding(8, 0)]] Texture3D<float>  BrickAtlas1   : register(t8);
[[vk::binding(8, 0)]] SamplerState      BrickSampler1 : register(s8);
[[vk::binding(10, 0)]] Texture3D<float>  BrickAtlas2   : register(t10);
[[vk::binding(10, 0)]] SamplerState      BrickSampler2 : register(s10);

#if HAS_MESH
// binding 12: the Forward reverse-Z mesh depth (SAMPLED, `.Load` unfiltered fetch — no sampler
// consumed). Declared ONLY in the HAS_MESH variant; the SDFONLY variant's layout still reserves
// the slot (bound-but-unread, the R2 contract), just never references it in SPIR-V.
Texture2D<float> gForwardDepth : register(t12);
#endif

#if VIEWT
// binding 13: the TAA `gViewT` ray-parameter lane (`core.viewt[fi]`, STORAGE, r32f). Declared
// ONLY in the VIEWT variants — see this file's header doc for the producer matrix (on a
// TAA-armed SDF-carrying VB leg this marcher is the composite and the SOLE gViewT producer,
// the `sdf_gbuffer_composite.hlsl` u8 precedent). The no-VIEWT variants' shared layout still
// reserves the slot (bound-but-unread, the SAME R2 contract binding 12 establishes).
// `[[vk::image_format]]` pins the `OpTypeImage` format (`shaderStorageImageWriteWithoutFormat`
// is OFF at device creation).
[[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(u13);

// The background sentinel (mirrors `viewt_from_depth_rz.comp.hlsl` / the deferred marcher's
// own `1.0e30`): background pixels reproject the point-at-infinity `(rd, 0)` inside the TAA
// resolve.
static const float VIEWT_BG = 1.0e30;
#endif

// The 40-byte push constant — see this file's header doc for the field table.
[[vk::push_constant]] struct PushConstants {
    uint   extent_w;        // offset 0
    uint   extent_h;        // offset 4
    float  view_z_a;        // offset 8  -- HAS_MESH reverse-Z decode A
    float  view_z_b;        // offset 12 -- HAS_MESH reverse-Z decode B
    float3 light_dir;       // offset 16 -- primary directional light direction (un-normalized)
    uint   brick_enabled;   // offset 28 -- M1 empty-skip gate; 0 = OFF
    uint   brick_trilinear; // offset 32 -- M2 trilinear+cubic gate; 0 = OFF
    uint   brick_levels;    // offset 36 -- M4 clip-map level count; 0 = OFF
} pc;

// Sphere-trace tuning (mirrors `sdf_gbuffer_composite.hlsl`'s §S2 march budget — the SDF field
// authority is shared across every render path, Principle 0, so the scene-scale constants match).
static const float EPS   = 0.001;
static const float T_MAX = 10.0;
static const uint  MAX_IT = 128u;

// A1 tuning (owner defaults — mirror `sdf_gbuffer_composite.hlsl`'s own consts).
static const float SHADOW_K           = 8.0;
static const float SHADOW_MINT        = 16.0 * GRAD_H;
static const float SHADOW_MINT_STEP   = 16.0 * GRAD_H;
static const float SHADOW_HIT_EPS     = 2.0 * EPS;
static const float SHADOW_NDOTL_EPS   = 0.0;
static const float SHADOW_NORMAL_BIAS = 0.02;

// The fallback material id (mirrors `sdf_gbuffer_composite.hlsl`).
static const uint DEFAULT_MATERIAL_ID = 0u;

// A1: clamped-step Quilez BASIC soft shadow — VERBATIM copy of `sdf_gbuffer_composite.hlsl`'s
// `sdf_soft_shadow` (the dot/early-return preamble is hand-written framing; the loop+tail is the
// SAME `boyko_shaderdsl::shadow`-generated span, pinned by `sdf_field_edsl_sync.rs` against THIS
// file too).
float sdf_soft_shadow(float3 p, float3 n, float3 L) {
    if (dot(n, L) <= SHADOW_NDOTL_EPS) {
        return 0.0; // surface faces away from the light — fully shadowed (and skips the march)
    }
    // === GENERATED sdf_soft_shadow BEGIN (boyko_shaderdsl::emit) ===
    float res = 1.0;
    float t = SHADOW_MINT;
    [loop]
    for (uint i = 0u; i < MAX_IT; ++i) {
        float d = field_distance(p + L * t);
        res = min(res, SHADOW_K * d / t);
        if (d < SHADOW_HIT_EPS) {
            return 0.0;
        }
        t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);
        if (t > T_MAX) {
            break;
        }
    }
    return clamp(res, 0.0, 1.0);
    // === GENERATED sdf_soft_shadow END ===
}

// ATTRIBUTE an SDF hit point `p` to the nearest edit's material id via an argmin over the edit
// list (VERBATIM copy of `sdf_gbuffer_composite.hlsl`'s `pick_material_id` — a read-only
// consumer of the FROZEN field, never touching it).
uint pick_material_id(float3 p) {
    uint n = min(Buf[0], MAX_SDF_EDITS);
    if (n == 0u) {
        return DEFAULT_MATERIAL_ID;
    }
    float best_d = FAR;
    uint best_id = DEFAULT_MATERIAL_ID;
    [loop]
    for (uint i = 0u; i < n; ++i) {
        Edit e = load_edit(i);
        float d = abs(edit_distance(e, p));
        if (d < best_d) {
            best_d = d;
            uint base = HEADER_BASE + i * SDF_EDIT_WORDS;
            best_id = asuint(Buf[base + 3u]);
        }
    }
    return best_id;
}

// --- SDF brick-atlas M1: the EMPTY-SPACE-SKIP helpers (VERBATIM copy) ---------------------

static const uint BRICK_EMPTY_OUTSIDE = 0u;
static const uint BRICK_EMPTY_INSIDE  = 1u;
static const uint BRICK_SURFACE       = 2u;
static const float BRICK_EXIT_EPS = 1.0e-4;
static const uint BRICK_OUTSIDE_GRID = 0xFFFFFFFFu;

// === GENERATED dist_to_brick_exit BEGIN ===
// MACHINE-GENERATED by `boyko_shaderdsl::emit::emit_hlsl_dist_to_brick_exit()`. DO NOT
// HAND-EDIT: re-run `cargo run -p boyko_shaderdsl --features emit --bin emit_field` and
// re-splice between these sentinels (the `sdf_field_edsl_sync` test guards drift, pinned
// against THIS file in addition to `sdf_gbuffer_composite.hlsl`).
float dist_to_brick_exit(float3 p, float3 rd, float3 cell_min, float bw) {
    float exit = 1.0e30;
    [unroll]
    for (uint a = 0u; a < 3u; ++a) {
        float t0 = rd[a];
        float t1 = cell_min[a];
        float t2 = t1 + bw;
        if (abs(t0) <= BRICK_EXIT_EPS) {
            continue;
        }
        float t3 = 1.0 / t0;
        float t4 = (t1 - p[a]) * t3;
        float t5 = (t2 - p[a]) * t3;
        float t6 = max(t4, t5);
        exit = min(exit, t6);
    }
    return (exit < BRICK_EXIT_EPS) ? BRICK_EXIT_EPS : exit;
}
// === GENERATED dist_to_brick_exit END ===

// === GENERATED brick_cell_class BEGIN ===
// Machine-generated by `boyko_shaderdsl::emit::emit_hlsl_brick_cell_class`. DO NOT HAND-EDIT —
// re-run and re-splice (see `dist_to_brick_exit`'s doc above). The three call sites
// (PointerGrid/1/2) are UNTOUCHED.
uint brick_cell_class(StructuredBuffer<uint> grid, float3 origin, float bw, uint3 dims,
                      float3 p, out float3 cell_min) {
    float3 rel = (p - origin) / bw;
    cell_min = origin;
    if (rel.x < 0.0 || rel.y < 0.0 || rel.z < 0.0) {
        return BRICK_OUTSIDE_GRID;
    }
    uint ix = (uint)rel.x;
    uint iy = (uint)rel.y;
    uint iz = (uint)rel.z;
    if (ix >= dims.x || iy >= dims.y || iz >= dims.z) {
        return BRICK_OUTSIDE_GRID;
    }
    uint idx = ix + iy * dims.x + iz * dims.x * dims.y;
    cell_min = origin + float3(ix, iy, iz) * bw;
    return grid[idx];
}
// === GENERATED brick_cell_class END ===

// --- SDF brick-atlas M2: the trilinear+JCGT-cubic SURFACE-brick path (VERBATIM copy) -------

// === GENERATED decode_snorm8 BEGIN ===
float m2_decode(float n, float band_half) {
    float t0 = n * band_half;
    return t0;
}
// === GENERATED decode_snorm8 END ===

float m2_corner(Texture3D<float> atlas, SamplerState atlas_smp,
                float3 tile_org, uint cx, uint cy, uint cz, float inv_atlas, float band_half) {
    float3 uvw = (float3(tile_org.x + (float)cx,
                         tile_org.y + (float)cy,
                         tile_org.z + (float)cz) + 0.5) * inv_atlas;
    float n = atlas.SampleLevel(atlas_smp, uvw, 0.0).r;
    return m2_decode(n, band_half);
}

uint m2_clamp_index(float g) {
    if (g <= 0.0) {
        return 0u;
    }
    uint i = (uint)g;
    return (i >= M2_BRICK_ALLOC - 1u) ? (M2_BRICK_ALLOC - 2u) : i;
}

// === GENERATED m2_cubic_eval BEGIN ===
float m2_cubic_eval(float4 c, float t) {
    float t0 = c.w * t;
    float t1 = t0 + c.z;
    float t2 = t1 * t;
    float t3 = t2 + c.y;
    float t4 = t3 * t;
    float t5 = t4 + c.x;
    return t5;
}
// === GENERATED m2_cubic_eval END ===

// === GENERATED m2_jcgt_cubic_coeffs BEGIN ===
float4 m2_jcgt_cubic_coeffs(float s[8], float3 a, float3 b) {
    float t0 = s[1] - s[0];
    float t1 = s[2] - s[0];
    float t2 = s[4] - s[0];
    float t3 = s[3] - s[1];
    float t4 = t3 - s[2];
    float t5 = t4 + s[0];
    float t6 = s[6] - s[2];
    float t7 = t6 - s[4];
    float t8 = t7 + s[0];
    float t9 = s[5] - s[1];
    float t10 = t9 - s[4];
    float t11 = t10 + s[0];
    float t12 = s[7] - s[3];
    float t13 = t12 - s[5];
    float t14 = t13 - s[6];
    float t15 = t14 + s[1];
    float t16 = t15 + s[2];
    float t17 = t16 + s[4];
    float t18 = t17 - s[0];
    float t19 = t0 * a.x;
    float t20 = s[0] + t19;
    float t21 = t1 * a.y;
    float t22 = t20 + t21;
    float t23 = t2 * a.z;
    float t24 = t22 + t23;
    float t25 = t5 * a.x;
    float t26 = t25 * a.y;
    float t27 = t24 + t26;
    float t28 = t8 * a.y;
    float t29 = t28 * a.z;
    float t30 = t27 + t29;
    float t31 = t11 * a.z;
    float t32 = t31 * a.x;
    float t33 = t30 + t32;
    float t34 = t18 * a.x;
    float t35 = t34 * a.y;
    float t36 = t35 * a.z;
    float t37 = t33 + t36;
    float t38 = t0 * b.x;
    float t39 = t1 * b.y;
    float t40 = t38 + t39;
    float t41 = t2 * b.z;
    float t42 = t40 + t41;
    float t43 = a.x * b.y;
    float t44 = a.y * b.x;
    float t45 = t43 + t44;
    float t46 = t5 * t45;
    float t47 = t42 + t46;
    float t48 = a.y * b.z;
    float t49 = a.z * b.y;
    float t50 = t48 + t49;
    float t51 = t8 * t50;
    float t52 = t47 + t51;
    float t53 = a.z * b.x;
    float t54 = a.x * b.z;
    float t55 = t53 + t54;
    float t56 = t11 * t55;
    float t57 = t52 + t56;
    float t58 = a.x * a.y;
    float t59 = t58 * b.z;
    float t60 = a.x * b.y;
    float t61 = t60 * a.z;
    float t62 = t59 + t61;
    float t63 = b.x * a.y;
    float t64 = t63 * a.z;
    float t65 = t62 + t64;
    float t66 = t18 * t65;
    float t67 = t57 + t66;
    float t68 = t5 * b.x;
    float t69 = t68 * b.y;
    float t70 = t8 * b.y;
    float t71 = t70 * b.z;
    float t72 = t69 + t71;
    float t73 = t11 * b.z;
    float t74 = t73 * b.x;
    float t75 = t72 + t74;
    float t76 = a.x * b.y;
    float t77 = t76 * b.z;
    float t78 = b.x * a.y;
    float t79 = t78 * b.z;
    float t80 = t77 + t79;
    float t81 = b.x * b.y;
    float t82 = t81 * a.z;
    float t83 = t80 + t82;
    float t84 = t18 * t83;
    float t85 = t75 + t84;
    float t86 = t18 * b.x;
    float t87 = t86 * b.y;
    float t88 = t87 * b.z;
    return float4(t37, t67, t85, t88);
}
// === GENERATED m2_jcgt_cubic_coeffs END ===

// === GENERATED m2_regula_falsi BEGIN ===
float m2_regula_falsi(float4 c, float lo, float hi, float f_lo, float f_hi) {
    float mid = lo;
    [loop]
    for (uint i = 0u; i < M2_MARMITT_ITERS; ++i) {
        float denom = f_hi - f_lo;
        mid = (abs(denom) > 1.0e-30) ? (lo - f_lo * (hi - lo) / denom) : (0.5 * (lo + hi));
        float f_mid = m2_cubic_eval(c, mid);
        if (abs(f_mid) <= M2_CUBIC_ROOT_EPS || hi - lo <= M2_CUBIC_ROOT_EPS) {
            return mid;
        }
        if (f_lo * f_mid <= 0.0) {
            hi = mid;
            f_hi = f_mid;
        } else {
            lo = mid;
            f_lo = f_mid;
        }
    }
    return mid;
}
// === GENERATED m2_regula_falsi END ===

float m2_marmitt_root(float4 c, float t0, float t1) {
    if (t1 <= t0) {
        return -1.0;
    }
    float qa = 3.0 * c.w;
    float qb = 2.0 * c.z;
    float qc = c.y;

    float e0 = t1;
    float e1 = t1;
    bool have0 = false;
    bool have1 = false;

    float disc = qb * qb - 4.0 * qa * qc;
    if (abs(qa) > 1.0e-30 && disc > 0.0) {
        float sq = sqrt(disc);
        float q = -0.5 * (qb + (qb >= 0.0 ? 1.0 : -1.0) * sq);
        float r0 = q / qa;
        float r1 = (abs(q) > 1.0e-30) ? (qc / q) : r0;
        if (r0 > r1) { float tmp = r0; r0 = r1; r1 = tmp; }
        if (r0 > t0 && r0 < t1) { e0 = r0; have0 = true; }
        if (r1 > t0 && r1 < t1) {
            if (have0) { e1 = r1; have1 = true; }
            else { e0 = r1; have0 = true; }
        }
    }

    float lo = t0;
    float f_lo = m2_cubic_eval(c, lo);
    float splits[3];
    splits[0] = have0 ? e0 : t1;
    splits[1] = have1 ? e1 : t1;
    splits[2] = t1;
    [unroll]
    for (uint i = 0u; i < 3u; ++i) {
        float hi = splits[i];
        if (hi <= lo) {
            continue;
        }
        float f_hi = m2_cubic_eval(c, hi);
        if (f_lo == 0.0) {
            return lo;
        }
        if (f_lo * f_hi <= 0.0) {
            return m2_regula_falsi(c, lo, hi, f_lo, f_hi);
        }
        lo = hi;
        f_lo = f_hi;
        if (hi >= t1) {
            break;
        }
    }
    return -1.0;
}

bool m2_brick_span(float3 p, float3 rd, float3 cell_min, float brick_world, out float t_enter, out float t_exit) {
    // === GENERATED m2_brick_span BEGIN ===
    float tmin = 0.0;
    float tmax = 1.0e30;
    [unroll]
    for (uint a = 0u; a < 3u; ++a) {
        float lo = cell_min[a];
        float hi = lo + brick_world;
        if (abs(rd[a]) <= 1.0e-20) {
            if (p[a] < lo || p[a] > hi) {
                t_enter = 1.0;
                t_exit = 0.0;
                return false;
            }
            continue;
        }
        float inv = 1.0 / rd[a];
        float t1 = (lo - p[a]) * inv;
        float t2 = (hi - p[a]) * inv;
        if (t1 > t2) {
            float tmp = t1;
            t1 = t2;
            t2 = tmp;
        }
        tmin = max(tmin, t1);
        tmax = min(tmax, t2);
    }
    t_enter = tmin;
    t_exit = tmax;
    return tmax > tmin;
    // === GENERATED m2_brick_span END ===
}

float m2_brick_cubic_hit(Texture3D<float> atlas, SamplerState atlas_smp,
                         float3 ro_v, float3 rd_v, float t_enter, float t_exit,
                         float3 tile_org, float inv_atlas, float band_half) {
    if (t_exit <= t_enter) {
        return -1.0;
    }
    const uint W = M2_BRICK_ALLOC;
    // === GENERATED m2_brick_cubic_hit BEGIN ===
    float t = t_enter;
    int cell[3];
    int step[3];
    float t_next[3];
    float t_delta[3];
    [unroll]
    for (uint axis = 0u; axis < 3u; ++axis) {
        float g_entry = ro_v[axis] + rd_v[axis] * t + M2_APRON - 0.5 + M2_ATLAS_BIAS;
        int c0 = (int)m2_clamp_index(g_entry);
        cell[axis] = c0;
        if (rd_v[axis] > 0.0) {
            step[axis] = 1;
            float boundary = (float)(c0 + 1);
            t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
            t_delta[axis] = 1.0 / rd_v[axis];
        } else {
            if (rd_v[axis] < 0.0) {
                step[axis] = -1;
                float boundary = (float)c0;
                t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
                t_delta[axis] = -1.0 / rd_v[axis];
            } else {
                step[axis] = 0;
                t_next[axis] = 1.0e30;
                t_delta[axis] = 1.0e30;
            }
        }
    }
    [loop]
    for (uint iter = 0u; iter < M2_MAX_CELLS; ++iter) {
        uint cx = min((uint)max(cell[0], 0), W - 2u);
        uint cy = min((uint)max(cell[1], 0), W - 2u);
        uint cz = min((uint)max(cell[2], 0), W - 2u);
        float s[8];
        s[0] = m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half);
        s[1] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy, cz, inv_atlas, band_half);
        s[2] = m2_corner(atlas, atlas_smp, tile_org, cx, cy + 1u, cz, inv_atlas, band_half);
        s[3] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy + 1u, cz, inv_atlas, band_half);
        s[4] = m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz + 1u, inv_atlas, band_half);
        s[5] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy, cz + 1u, inv_atlas, band_half);
        s[6] = m2_corner(atlas, atlas_smp, tile_org, cx, cy + 1u, cz + 1u, inv_atlas, band_half);
        s[7] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy + 1u, cz + 1u, inv_atlas, band_half);
        float t_cell_exit = min(min(min(t_next[0], t_next[1]), t_next[2]), t_exit);
        float seg_lo = max(t, t_enter);
        float seg_hi = min(t_cell_exit, t_exit);
        if (seg_hi > seg_lo) {
            float3 lo_g = float3(ro_v[0] + rd_v[0] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cx, ro_v[1] + rd_v[1] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cy, ro_v[2] + rd_v[2] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cz);
            float4 coeffs = m2_jcgt_cubic_coeffs(s, lo_g, rd_v);
            float local_t = m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo);
            if (local_t >= 0.0) {
                return seg_lo + local_t;
            }
        }
        if (t_cell_exit >= t_exit) {
            break;
        }
        uint axis = (t_next[0] <= t_next[1] && t_next[0] <= t_next[2]) ? 0u : ((t_next[1] <= t_next[2]) ? 1u : 2u);
        t = t_next[axis];
        cell[axis] += step[axis];
        t_next[axis] += t_delta[axis];
        if (step[axis] == 0 || cell[axis] < 0 || (uint)cell[axis] >= W - 1u) {
            break;
        }
    }
    return -1.0;
    // === GENERATED m2_brick_cubic_hit END ===
}

// The M2 SURFACE-brick step (VERBATIM copy of `sdf_gbuffer_composite.hlsl`'s `m2_surface_hit`
// including its GENERATED analytic-residual signed-refine tail).
bool m2_surface_hit(M4Level lvl, Texture3D<float> atlas, SamplerState atlas_smp,
                    float3 ro, float3 rd, float t_world, out float hit_t) {
    hit_t = t_world;
    float3 origin = lvl.origin_brick_world.xyz;
    float brick_world = lvl.origin_brick_world.w;
    uint3 dims = (uint3)lvl.dims_atlas_dim.xyz;
    float band_half = lvl.band_voxel_inv_atlas.x;
    float voxel_size = lvl.band_voxel_inv_atlas.y;
    float inv_atlas = lvl.band_voxel_inv_atlas.z;

    float3 p = ro + rd * t_world;
    float3 rel = (p - origin) / brick_world;
    if (rel.x < 0.0 || rel.y < 0.0 || rel.z < 0.0) {
        return false;
    }
    uint tx = (uint)rel.x;
    uint ty = (uint)rel.y;
    uint tz = (uint)rel.z;
    if (tx >= dims.x || ty >= dims.y || tz >= dims.z) {
        return false;
    }
    float3 cell_min = origin + float3((float)tx, (float)ty, (float)tz) * brick_world;
    // M5 toroidal slot addressing (mirrors `sdf_gbuffer_composite.hlsl` bit-for-bit).
    int3 origin_cell = (int3)round(origin / brick_world);
    int3 world_cell = origin_cell + int3((int)tx, (int)ty, (int)tz);
    const int WRAP_BIAS = (int)(M2_GRID_DIM * 1024u);
    uint3 slot = (uint3)((world_cell + WRAP_BIAS) % (int)M2_GRID_DIM);
    float3 tile_org = float3((float)(slot.x * M2_BRICK_ALLOC),
                             (float)(slot.y * M2_BRICK_ALLOC),
                             (float)(slot.z * M2_BRICK_ALLOC));

    float t_enter, t_exit;
    if (!m2_brick_span(p, rd, cell_min, brick_world, t_enter, t_exit)) {
        return false;
    }
    float3 ro_v = (p - cell_min) / voxel_size;
    float3 rd_v = rd / voxel_size;

    float local = m2_brick_cubic_hit(atlas, atlas_smp, ro_v, rd_v, t_enter, t_exit, tile_org, inv_atlas, band_half);
    if (local < 0.0) {
        return false;
    }

    float cand_t = t_world + local;

    // === GENERATED m2_surface_hit_refine BEGIN ===
    float rt = cand_t;
    [loop]
    for (uint i = 0u; i < M2_REFINE_ITERS; ++i) {
        float d = field_distance(ro + rd * rt);
        if (abs(d) < EPS) {
            hit_t = rt;
            return true;
        }
        float step = M2_REFINE_RELAX * d;
        rt = rt + step;
        if (rt < 0.0 || rt > T_MAX) {
            break;
        }
    }
    return false;
    // === GENERATED m2_surface_hit_refine END ===
}

// SDF brick-atlas M4 (clip-map LOD): selects the FINEST enclosing clip-map level for world point
// `p`, or -1 outside every active level (VERBATIM copy of `sdf_gbuffer_composite.hlsl`'s
// `select_level`).
int select_level(float3 p) {
    // === GENERATED select_level BEGIN ===
    [unroll]
    for (uint L = 0u; L < BRICK_LEVELS; ++L) {
        if (L >= pc.brick_levels) {
            break;
        }
        float3 o = m2_levels[L].origin_brick_world.xyz;
        float bw = m2_levels[L].origin_brick_world.w;
        float3 hi = o + m2_levels[L].dims_atlas_dim.xyz * bw;
        if (all(p >= o) && all(p < hi)) {
            return (int)L;
        }
    }
    return -1;
    // === GENERATED select_level END ===
}

// --- Shading (cloned from `forward_opaque.fs.hlsl` — see this file's header doc) -----------

// `pbr_lighting.hlsli`'s INCLUDE CONTRACT precondition: `PI` + `LIGHT_UP` in scope first.
static const float PI = 3.14159265358979323846;
static const float3 LIGHT_UP = float3(0.0, 1.0, 0.0);

#include "pbr_lighting.hlsli"
#include "light_table.hlsli"

// --- Set 1 (shadow): a VERBATIM copy of `forward_opaque.fs.hlsl`'s own Set-1 block, so the SAME
// physical descriptor set (`ForwardTargets::set1`) binds to both the mesh raster and this pass.

static const uint MAX_CASCADES = 4u;
struct CascadeData {
    float4x4 view_proj;
    float    split_far;
    float    texel_size;
    float2   _pad;
};
[[vk::binding(0, 1)]] Texture2DArray<float> gCsm : register(t12);
[[vk::binding(0, 1)]] SamplerComparisonState gCsmCmp : register(s12);
[[vk::binding(1, 1)]] cbuffer CsmCascades {
    CascadeData gCascades[MAX_CASCADES];
    uint gCsmActive;
    uint gCsmMode;
    uint gCsmPcfKernel;   // rung E1: the CsmPcfKernel word — `csm_pcf_disc` branches on it
    uint _gCsmPad;
};

static const uint M_SLOTS = 16u;
struct FaceTransform {
    float4x4 view_proj;
    float3   light_pos;
    float    inv_range;
};
[[vk::binding(2, 1)]] Texture2DArray<float> gShadowAtlas : register(t14);
[[vk::binding(2, 1)]] SamplerComparisonState gShadowAtlasCmp : register(s14);
[[vk::binding(3, 1)]] cbuffer ShadowAtlas {
    FaceTransform gFaces[M_SLOTS];
    uint gAtlasActive;
    uint gAtlasMode;
    uint2 _gAtlasPad;
};

#include "shadow_apply.hlsli"

// --- Duplicated pure helpers (VERBATIM copy of `forward_opaque.fs.hlsl`'s own span — see that
// file's "Duplicated (not shared) pure helpers" doc for the rationale) ----------------------

float2 env_brdf_approx(float roughness, float NoV) {
    const float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
    const float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
    float4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * NoV)) * r.x + r.y;
    return float2(-1.04, 1.04) * a004 + r.zw;
}

static const float SUN_KERNEL_EXPONENT_MIN = 1.0;
static const float SUN_KERNEL_EXPONENT_MAX = 2048.0;

float sun_kernel_exponent(float alpha) {
    float n = 2.0 / max(alpha * alpha, 1e-6) - 2.0;
    return clamp(n, SUN_KERNEL_EXPONENT_MIN, SUN_KERNEL_EXPONENT_MAX);
}

float sun_kernel(float3 dir, float3 sun_dir, float alpha) {
    float c = saturate(dot(dir, sun_dir));
    return pow(c, sun_kernel_exponent(alpha));
}

static const float SUN_ENV_WEIGHT = 1.0;

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    uint w = pc.extent_w;
    uint h = pc.extent_h;
    if (idx >= w * h) {
        return;
    }
    uint px = idx % w;
    uint py = idx / w;

    float3 ro;
    float3 rd;
    generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);

    // HAS_MESH ownership gate (Decision 4's consumer half — see this file's header doc).
    // `ro == cam_eye` for a perspective ray (`ray_gen.hlsli`'s own contract), so the mesh's
    // view-space depth converts to the SAME euclidean ray-parameter metric the march uses via
    // `t_mesh = view_z_mesh / dot(cam_forward, rd)` (the identical conversion
    // `deferred_pbr.hlsl`'s CSM cascade-select already performs the other way, `view_z = dot(rd,
    // cam_forward) * view_t`).
#if HAS_MESH
    float depth_mesh = gForwardDepth.Load(int3((int)px, (int)py, 0)).r;
    // SAFETY (memory ordering, not unsafe): this is a plain arithmetic read of a value the
    // graph's derived DEPTH_ATTACHMENT_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL barrier (recorded at
    // this pass) makes visible after `forward_opaque`'s depth write — no unsynchronized access.
    float view_z_mesh = pc.view_z_b / (depth_mesh - pc.view_z_a);
    float cos_fwd = dot(cam_forward.xyz, rd);
    float t_mesh = view_z_mesh / max(cos_fwd, 1.0e-4);
#else
    float t_mesh = 1.0e30;
#endif

    // Plain sphere-trace with the M1/M2/M4 brick acceleration folded in (Render B1's
    // over-relaxation is NOT copied this rung — see this file's header doc).
    float t = 0.0;
    bool hit = false;
    [loop]
    for (uint it = 0u; it < MAX_IT; ++it) {
        if (t >= t_mesh) {
            break;
        }
        float3 p = ro + rd * t;

        if (pc.brick_enabled != 0u) {
            int lvl = select_level(p);
            if (lvl >= 0) {
                float3 cell_min;
                uint cls;
                float bw;
                if (lvl == 0) {
                    cls = brick_cell_class(PointerGrid, m2_levels[0].origin_brick_world.xyz,
                                           m2_levels[0].origin_brick_world.w,
                                           (uint3)m2_levels[0].dims_atlas_dim.xyz, p, cell_min);
                    bw = m2_levels[0].origin_brick_world.w;
                } else if (lvl == 1) {
                    cls = brick_cell_class(PointerGrid1, m2_levels[1].origin_brick_world.xyz,
                                           m2_levels[1].origin_brick_world.w,
                                           (uint3)m2_levels[1].dims_atlas_dim.xyz, p, cell_min);
                    bw = m2_levels[1].origin_brick_world.w;
                } else {
                    cls = brick_cell_class(PointerGrid2, m2_levels[2].origin_brick_world.xyz,
                                           m2_levels[2].origin_brick_world.w,
                                           (uint3)m2_levels[2].dims_atlas_dim.xyz, p, cell_min);
                    bw = m2_levels[2].origin_brick_world.w;
                }
                if (cls == BRICK_EMPTY_OUTSIDE) {
                    t += dist_to_brick_exit(p, rd, cell_min, bw);
                    if (t > T_MAX) {
                        break;
                    }
                    continue;
                }
            }
        }

        if (pc.brick_trilinear != 0u) {
            int lvl = select_level(p);
            if (lvl >= 0) {
                float m2_hit_t;
                bool m2_hit;
                if (lvl == 0) {
                    m2_hit = m2_surface_hit(m2_levels[0], BrickAtlas,  BrickSampler,  ro, rd, t, m2_hit_t);
                } else if (lvl == 1) {
                    m2_hit = m2_surface_hit(m2_levels[1], BrickAtlas1, BrickSampler1, ro, rd, t, m2_hit_t);
                } else {
                    m2_hit = m2_surface_hit(m2_levels[2], BrickAtlas2, BrickSampler2, ro, rd, t, m2_hit_t);
                }
                if (m2_hit) {
                    hit = true;
                    t = m2_hit_t;
                    break;
                }
            }
        }

        float d = sdf(p);
        if (d < EPS) {
            hit = true;
            break;
        }
        t = t + d;
        if (t > T_MAX) {
            break;
        }
    }

    bool sdf_owns = hit && t < t_mesh;
    if (!sdf_owns) {
        // Misses / mesh-occluded pixels write NOTHING — the sky/mesh color `forward_opaque`
        // already painted this frame stands (the deferred marcher's own ownership-gate
        // contract, mirrored here).
#if VIEWT
        // ... except the gViewT lane, written for EVERY in-bounds pixel (the exactly-once
        // discipline): the decoded mesh `t_mesh` when a rasterized surface owns the pixel,
        // else the background sentinel. Reverse-Z occupancy: the depth CLEAR is 0.0 ("nothing
        // drawn" under GREATER), so any `depth_mesh > 0.0` is a real mesh surface —
        // `viewt_from_depth_rz.comp.hlsl`'s own `has_mesh` test.
#if HAS_MESH
        gViewT[uint2(px, py)] = (depth_mesh > 0.0) ? t_mesh : VIEWT_BG;
#else
        gViewT[uint2(px, py)] = VIEWT_BG;
#endif
#endif
        return;
    }

    float3 p = ro + rd * t;
    float3 n = sdf_normal(p);
    uint mat_id = pick_material_id(p);
    MaterialGpu m = Materials[mat_id];

    float3 base = m.base_color.rgb;
    float metallic = m.mrr.x;
    float roughness = clamp(m.mrr.y, 0.045, 1.0);
    float reflectance = m.mrr.z;
    float3 emissive = m.emissive.rgb;
    float a = roughness * roughness;

    float3 v = normalize(cam_eye.xyz - p);
    float NoV = max(dot(n, v), 1e-4);

    float3 f0 = lerp(0.16 * reflectance * reflectance, base, metallic);
    float3 diffuse_color = base * (1.0 - metallic);

    float2 dfg_v = env_brdf_approx(roughness, NoV);
    float Ess = max(dfg_v.x + dfg_v.y, 1e-4);
    float3 energy_comp = 1.0 + f0 * (1.0 / Ess - 1.0);

    Surface surf;
    surf.n = n;
    surf.NoV = NoV;
    surf.a = a;
    surf.f0 = f0;
    surf.diffuse_color = diffuse_color;
    surf.energy_comp = energy_comp;

    float3 R = reflect(-v, n);
    float hemi = dot(n, LIGHT_UP) * 0.5 + 0.5;

    // v1 scope cut (mirrors `forward_opaque.fs.hlsl`'s own "no SSAO/DDGI consumer" note): no
    // SDF-side AO term is applied this rung — `ao_final` stays the Forward v1 mesh-leg constant.
    float ao_final = 1.0;
    float spec_ao = saturate(pow(NoV + ao_final, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao_final);

    LightHeader H = load_light_header(LightBuf);
    uint csm_mode = load_csm_mode(LightBuf);
    uint punctual_shadow_mode = load_punctual_shadow_mode(LightBuf);

    float3 lit_direct = float3(0.0, 0.0, 0.0);
    float3 ambient = float3(0.0, 0.0, 0.0);
    bool primary_dir_seen = false;

    // L0a: directionals + sky (ALL-LIGHTS -- no froxel cull this rung, mirrors plain `Forward`'s
    // own base compile; see this file's header doc).
    for (uint i = 0u; i < H.l0a_count; ++i) {
        LightElem L = load_light(LightBuf, i);
        if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
            float3 l = normalize(L.dir);
            float NoL = max(dot(n, l), 0.0);
            // The ONE shading difference from `forward_opaque.fs.hlsl`: the primary's
            // visibility starts at THIS pass's own `sdf_soft_shadow` analytic march (the SDF's
            // baked self-shadow) instead of the mesh leg's implicit `1.0`, then min-combines
            // CSM in exactly as `deferred_pbr.hlsl`'s `is_sdf_lit` branch does for the primary.
            float vis = 1.0;
            if (!primary_dir_seen) {
                primary_dir_seen = true;
                vis = sdf_soft_shadow(p + n * SHADOW_NORMAL_BIAS, n, l);
                if (csm_mode != CSM_MODE_OFF && NoL > 0.0) {
                    float view_z = dot(cam_forward.xyz, p - cam_eye.xyz);
                    vis = min(vis, csm_visibility(p, n, view_z, NoL));
                }
            }
            PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
            lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * vis) * L.color;

            float sun_k = sun_kernel(R, l, a);
            float3 sun_spec_ambient = eval_pbr_sun_disc(surf, dfg_v, sun_k, L.color) * SUN_ENV_WEIGHT;
            ambient += sun_spec_ambient * spec_ao;
        } else if (light_kind(L) == LIGHT_KIND_SKY) {
            float3 sky_color = L.color;
            float3 ground_color = L.pos;
            ambient += eval_pbr_ambient_hemi(surf, R, dfg_v, sky_color, ground_color, hemi, ao_final, spec_ao);
        }
    }

    // L0b: the point/spot block (ALL-LIGHTS flat scan, TOKEN-FOR-TOKEN clone of
    // `forward_opaque.fs.hlsl`'s own non-FROXEL arm).
    for (uint j = H.l0a_count; j < H.light_count; ++j) {
        LightElem L = load_light(LightBuf, j);
        float3 toL = L.pos - p;
        float d2 = dot(toL, toL);
        float range2 = L.range * L.range;
        if (d2 > range2) {
            continue;
        }
        float inv_d = rsqrt(max(d2, 1e-8));
        float3 l = toL * inv_d;
        float win = saturate(1.0 - (d2 * d2) / (range2 * range2));
        float atten = (1.0 / max(d2, 1e-4)) * win * win;
        if (light_kind(L) == LIGHT_KIND_SPOT) {
            float2 cones = unpack_cones(L.cone_pack);
            float3 spot_dir = safe_normalize(L.dir);
            float cosA = dot(-l, spot_dir);
            float denom = max(cones.x - cones.y, 1e-4);
            float tt = saturate((cosA - cones.y) / denom);
            atten *= tt * tt;
        }

        float punctual_shadow = 1.0;
        if (punctual_shadow_mode != PUNCTUAL_SHADOW_MODE_OFF) {
            uint slot = light_atlas_slot(L.kind);
            if (slot != SLOT_NONE) {
                float pnol = max(dot(n, l), 0.0);
                if (light_kind(L) == LIGHT_KIND_SPOT) {
                    punctual_shadow = spot_atlas_visibility(slot, p, n, pnol);
                } else if (light_kind(L) == LIGHT_KIND_POINT) {
                    punctual_shadow = punctual_atlas_visibility(slot, p, n, pnol);
                }
            }
        }

        float NoL = max(dot(n, l), 0.0);
        PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
        lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * punctual_shadow) * atten * L.color;
    }

    // Same tail as `forward_opaque.fs.hlsl` / the deferred resolve: exposure LAST, THEN the
    // owner-selected tonemap, THEN the manual gamma-2.2 OETF.
    float3 lit = (lit_direct + ambient + emissive) * H.exposure;
    lit = tonemap_select(lit, load_tonemap_mode(LightBuf));
    lit = pow(lit, OETF_GAMMA_EXP);
    gLit[uint2(px, py)] = float4(clamp(lit, 0.0, 1.0), 1.0);
#if VIEWT
    // The SDF-owned half of the exactly-once gViewT discipline: the marched ray parameter `t`
    // (the SAME euclidean metric the `t_mesh` decode above converts into — the TAA resolve
    // reconstructs `P = ro + rd * t` on bitwise-identical `ray_gen.hlsli` rays).
    gViewT[uint2(px, py)] = t;
#endif
}
