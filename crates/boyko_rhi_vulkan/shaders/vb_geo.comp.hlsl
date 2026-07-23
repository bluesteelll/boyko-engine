// R9 VB geo/shade split plan (docs/R9-VB-SPLIT-PLAN.md, rung R9b, Section 5): the split's
// THIN-AUX GEOMETRY pass. One thread per pixel over `vb_id` (the SAME tid-linear dispatch
// shape `vb_resolve.comp.hlsl` uses): unpacks `vb_id` -> re-fetches the covered triangle's
// geometry (`vb_geom_fetch.hlsli`, the SAME Set-2 contract, `%tri_count`, bary, perspective-
// correct interp -- ZERO new math) -> writes the interpolated GEOMETRIC vertex normal
// (oct-encoded) + a material roughness scalar into `gThinNormal`. No lighting happens here --
// `vb_shade_split.comp.hlsl` RE-fetches + RE-shades independently (Decision 5's pure-VB
// re-derive-from-scratch model, unchanged by the split).
//
// # Sentinel / miss handling
//
// `instance_id == VB_ID_SENTINEL` marks a pixel `vb_raster` never covered (the sky background,
// or a future SDF-owned pixel): this pass WRITES NOTHING for such a pixel. Unlike `gLit`
// (which `vb_sky` pre-paints every frame, so "write nothing" means "the sky stands"),
// `gThinNormal` has NO prior writer this frame -- a background texel's first touch stays
// genuinely UNDEFINED. This is discard-legal: the framegraph's `ssao`-precedent seed
// (`add_image_seeded(ResSync::undefined())`) already establishes that an UNDEFINED-content
// image is a valid graph state, and NOTHING downstream ever samples a background
// `gThinNormal` texel (the SSAO gather only marches from a LIT center pixel, Â§5's
// `sdf_ssao.comp.hlsl` `-D VB_THIN=1` variant; the shade pass never reads `gThinNormal` at
// all -- it RE-fetches its own normal independently).
//
// # Geometry (cloned from `vb_resolve.comp.hlsl`'s own re-fetch -- see that file's header doc)
//
// After a valid fetch, the world normal is `vb_geom_fetch`'s interpolated GEOMETRIC vertex
// normal (NO tangent-space normal mapping -- v1 scope cut, see below) and the roughness scalar
// is the per-instance material's `Materials[mat].mrr.y`, clamped to the SAME `[0.045, 1.0]`
// fp32 floor `vb_resolve.comp.hlsl`/the deferred resolve apply. The oct-encode is
// `oct_encode` -- the SAME fold `gNormal`'s RG channels use in `gbuffer_mrt.fs.hlsl` /
// `sdf_gbuffer_composite.hlsl`. This codebase has NO shared `oct.hlsli`: `oct_encode`'s BODY
// is eDSL-single-sourced (`boyko_shaderdsl::oct::oct_encode_body`) and SPLICED verbatim into
// every consuming file between the `// === GENERATED ... BEGIN/END ===` sentinels (the
// established per-file-splice convention, not a `#include`) -- this file adds a THIRD splice
// site, unmodified from the other two.
//
// # Material / texture (v1 scope cut -- NO SampleGrad, NO Set-3)
//
// `vb_geom_fetch`'s reconstructed `uv` is ready (perspective-correct) but UNREAD here -- no
// bindless texture table is bound (mirrors `vb_resolve.comp.hlsl`'s own base-compile scope
// cut). No armed consumer reads a normal-mapped thin normal today, and SSAO/denoise on the
// GEOMETRIC normal is the standard real-time trade (docs/R9-VB-SPLIT-PLAN.md Â§5's rejected
// alternative: a SampleGrad'd thin normal, revisited with SSR).
//
// # `gThinNormal` packing (RGBA8, `[[vk::image_format("rgba8")]]`)
//
//   RG : the oct-encoded world vertex normal (`oct_encode`, above).
//   B  : the clamped material roughness scalar (`Materials[mat].mrr.y`), stored directly as an
//        UNORM channel value -- roughness is already a natural `[0, 1]` range, so no hi/lo
//        byte split is needed (unlike `gNormal`'s 16-bit material-id BA pack, which DOES need
//        one).
//   A  : `1.0`, a spare/marker channel -- unread by every consumer this rung (mirrors `gLit`'s
//        own unconditional `.a = 1.0` write). The Â§5 `sdf_ssao.comp.hlsl` `-D VB_THIN=1` SSAO
//        gather reads ONLY RG; nothing reads B/A yet (a later rung may read B for a
//        roughness-aware AO/denoise radius).
//
// # Bindings
//
//   Set 0 (`vb_layout0`, REUSED verbatim -- the CURRENT 8-binding shared layout object; the
//   R2 contract, bound-but-unread entries STATICALLY declared so this pipeline shares the
//   ONE physical `vb_layout0` descriptor-set-layout with `vb_resolve`/`vb_shade{,_tex}`):
//     b0/u0: StructuredBuffer<VbInstanceRow>      gVbInstances       READ (`vb_geom_fetch.hlsli`)
//     t1   : StructuredBuffer<PerInstanceMaterial> instance_materials READ (resolves `mat.id`
//                                                                       before indexing
//                                                                       `Materials`, below)
//     b2   : cbuffer Camera                                          READ (extent bounds guard)
//     t3   : StructuredBuffer<uint>                LightBuf          bound-but-unread
//     t4   : StructuredBuffer<MaterialGpu>          Materials        READ (`.mrr.y` roughness)
//     t5   : Texture2D<uint2>                       gVbId             READ (`.Load` unfiltered)
//     u6   : RWTexture2D<float4> (rgba8)             gLit             bound-but-unread
//     u7   : RWByteAddressBuffer                     gClassify         bound-but-unread
//                                                                      (`vb_classify_common.hlsli`
//                                                                       -- split displaces
//                                                                       classification, Â§0)
//   Set 1 (`vb_geo_aux_layout`, NEW): the thin-aux write targets this pass owns.
//     u0 (register u8) : RWTexture2D<float4> (rgba8) gThinNormal      WRITE, UNCONDITIONAL under
//                                                                      split (Â§1.2 guarantees
//                                                                      NORMAL is armed in every
//                                                                      split config)
//     u1 (register u9) : RWTexture2D<float2> (rg16f) gMotion          `#if MOTION` ONLY -- the
//                                                                      R9d `vb_geo_mv` variant
//                                                                      (`-D MOTION=1`); declared
//                                                                      but NOT written this rung
//                                                                      (no `main()` body yet)
//     b2 (register b10): cbuffer MotionCam                            `#if MOTION` ONLY -- the
//                                                                      R9d prev/curr unjittered
//                                                                      camera pair; declared but
//                                                                      unread this rung
//   Set 2 (geometry, UNCHANGED) -- `vb_geom_fetch.hlsli`'s own `gMeshVerts[]`/`gMeshIndices[]`/
//   `gMeshMeta`.
//
// # Push constant (64 bytes -- the geometry-fetch reprojection matrix, UNCHANGED shape from
// `vb_resolve.comp.hlsl`'s own)
//
//   offset 0: float4x4 view_proj   the SAME reverse-Z proj*view `vb_raster.vs.hlsl` used
//
// # Arming
//
// Recorded iff `path_vb_split()` (docs/R9-VB-SPLIT-PLAN.md Â§2 -- `path_is_vb() &&
// resolved_render_path.mesh_geo_shade_split`), paired 1:1 with `vb_shade_split.comp.hlsl`. On
// the fused arm (`path_vb_fused()`) this pass is NOT declared -- `vb_resolve`/`vb_shade{,_tex}`
// keep the fused classification/lit-producer selection untouched (Â§0's "split displaces
// classification" surgical re-gate).
//
// Compiled offline (hermetic build -- no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_geo.comp.hlsl -Fo vb_geo.comp.spv
//   (R9d MOTION variant, NOT YET WIRED -- the `#if MOTION` declarations above are authored now
//   so the R9d descriptor-set layout lands alongside the base one, but `main()` carries no
//   motion-write body yet): add `-D MOTION=1 -Fo vb_geo_mv.comp.spv`
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_geo.comp.spv

// binding 0 (`gVbInstances`, the `VbInstanceRow` SSBO) is declared inside `vb_geom_fetch.hlsli`
// itself (that file's own INCLUDE CONTRACT -- self-contained, needs nothing pre-declared).
#include "vb_pack.hlsli"
#include "vb_geom_fetch.hlsli"

// binding 1: the per-instance material payload -- byte-identical shape to
// `vb_resolve.comp.hlsl`'s own declaration. READ (`.id` resolves the `Materials` row below).
struct PerInstanceMaterial {
    float4 base_color;
    uint   id;
    uint3  _pad;
};
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;

// binding 2: the extent/camera UNIFORM block -- byte-identical shape to every other consumer's.
[[vk::binding(2, 0)]] cbuffer Camera {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};

// binding 3: the Lighting-L0 light table. Bound-but-unread (the R2 contract) -- this pass
// writes geometry only, no shading.
[[vk::binding(3, 0)]] StructuredBuffer<uint> LightBuf;

// binding 4: the material table -- byte-identical `MaterialGpu` shape to every other consumer.
// READ: `.mrr.y` is the roughness scalar this pass packs into `gThinNormal.b`.
struct MaterialGpu {
    float4 base_color;
    float4 mrr;
    float4 emissive;
};
[[vk::binding(4, 0)]] StructuredBuffer<MaterialGpu> Materials;

// binding 5: the `vb_id` raster output (R32G32_UINT) -- SAMPLED, unfiltered `.Load` fetch.
Texture2D<uint2> gVbId : register(t5);

// binding 6: the shared `lit` STORAGE image. Bound-but-unread (the R2 contract) -- this pass
// never writes `lit`.
[[vk::image_format("rgba8")]] RWTexture2D<float4> gLit : register(u6);

// binding 7: the packed classify buffer (`vb_classify_common.hlsli`'s own declaration).
// Bound-but-unread -- split displaces classification (Â§0), so this pass never touches it, but
// the descriptor stays declared for the ONE shared `vb_layout0` layout object.
#include "vb_classify_common.hlsli"

// --- Set 1 (`vb_geo_aux_layout`, NEW): the thin-aux write targets ---------------------------

[[vk::binding(0, 1)]] [[vk::image_format("rgba8")]] RWTexture2D<float4> gThinNormal : register(u8);

#if MOTION
// R9d (`vb_geo_mv`, `-D MOTION=1`): the per-pixel camera-reprojected motion vector. Declared
// now so the R9d descriptor-set layout is authored alongside the base one; NOT written by
// `main()` this rung (no motion-write body exists yet -- see this file's header doc).
[[vk::binding(1, 1)]] [[vk::image_format("rg16f")]] RWTexture2D<float2> gMotion : register(u9);

// R9d: the previous/current unjittered marcher-aligned view_proj pair the motion write will
// reproject against (camera-only, static-geometry motion -- the SDF leg's C6 semantics).
// Declared but unread this rung.
[[vk::binding(2, 1)]] cbuffer MotionCam : register(b10) {
    float4x4 mv_cur_view_proj;
    float4x4 mv_prev_view_proj;
};
#endif

// Octahedral-encode a unit normal into [0,1]^2 -- the SAME fold `gNormal`'s RG channels use
// (`gbuffer_mrt.fs.hlsl` / `sdf_gbuffer_composite.hlsl`'s own `oct_encode`). This codebase has
// no shared `oct.hlsli`; the BODY is eDSL-single-sourced
// (`boyko_shaderdsl::oct::oct_encode_body`) and SPLICED verbatim per consuming file (see this
// file's header doc) -- a THIRD splice site, unmodified from the other two.
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

// The 64-byte push constant -- the geometry-fetch reprojection matrix (the SAME shape
// `vb_resolve.comp.hlsl`'s own declares).
[[vk::push_constant]] struct PushConstants {
    float4x4 view_proj;
} pc;

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    uint w = img_w_raw;
    uint h = img_h_raw;
    if (idx >= w * h) {
        return;
    }
    uint px = idx % w;
    uint py = idx / w;

    // SAFETY (memory ordering, not unsafe): this is a plain read of a value the graph's derived
    // COLOR_ATTACHMENT_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL barrier (recorded at this pass) makes
    // visible after `vb_raster`'s color write -- no unsynchronized access.
    uint2 packed = gVbId.Load(int3((int)px, (int)py, 0));
    VbId id = vb_id_unpack(packed);
    if (id.instance_id == VB_ID_SENTINEL) {
        // Background: writes NOTHING. Unlike `gLit` (which `vb_sky` pre-paints every frame),
        // `gThinNormal` has no prior writer this frame -- the texel's first touch stays
        // genuinely UNDEFINED, which is discard-legal (this file's header doc) since nothing
        // downstream ever samples a background `gThinNormal` texel.
        return;
    }

    float2 pixel_xy = float2((float)px, (float)py) + 0.5; // pixel-CENTER, matching SV_Position
    float2 extent = float2((float)w, (float)h);
    VbGeomFetchResult geo = vb_geom_fetch(id.instance_id, id.raw_prim_id, pixel_xy, pc.view_proj, extent);

    float3 n = normalize(geo.world_normal);

    PerInstanceMaterial pm = instance_materials[id.instance_id];
    MaterialGpu m = Materials[pm.id];
    float roughness = clamp(m.mrr.y, 0.045, 1.0); // fp32 floor, mirrors vb_resolve/the deferred resolve

    float2 oct = oct_encode(n);
    gThinNormal[uint2(px, py)] = float4(oct.x, oct.y, roughness, 1.0);
}
