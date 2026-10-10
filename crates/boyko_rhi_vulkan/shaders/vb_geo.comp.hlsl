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
//     t3   : StructuredBuffer<uint>                LightBuf          bound-but-unread in the BASE
//                                                                      and MOTION variants; READ by
//                                                                      the `VB_SV0_TERM` variant
//                                                                      (mode word + the primary
//                                                                      directional's loads). DXC
//                                                                      STRIPS a declared-but-unread
//                                                                      StructuredBuffer, so this
//                                                                      slot is genuinely ABSENT
//                                                                      from `vb_geo.comp.spv` /
//                                                                      `vb_geo_mv.comp.spv`'s
//                                                                      reflected interface and
//                                                                      PRESENT in `vb_geo_sv0`'s --
//                                                                      Set 0's reflection is NOT
//                                                                      common across the three
//                                                                      (measured: 0 / 0 / 2
//                                                                      `OpDecorate %LightBuf`).
//                                                                      Harmless: the shared
//                                                                      `vb_layout0` DESCRIPTOR SET
//                                                                      LAYOUT is one object and
//                                                                      always binds it; a module
//                                                                      that does not statically use
//                                                                      a descriptor imposes no
//                                                                      requirement on it.
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
//                                                                      (`-D MOTION=1`); WRITTEN
//                                                                      once per non-sentinel
//                                                                      pixel (camera-reprojected
//                                                                      Delta-uv, this file's
//                                                                      `main()` body)
//     b2 (register b10): cbuffer MotionCam                            `#if MOTION` ONLY -- the
//                                                                      R9d prev/curr unjittered
//                                                                      camera pair; READ by the
//                                                                      motion-vector write below
//     u3 (register u11): RWTexture2D<float2> (rg8)   gSdfTerm         `#ifdef VB_SV0_TERM` ONLY --
//                                                                      the VB-SV0 DP6 term this
//                                                                      variant PRODUCES (R = soft
//                                                                      shadow visibility, G =
//                                                                      contact AO), written once
//                                                                      per covered pixel under the
//                                                                      wave-uniform mode gate
//     t4 (register t0) : StructuredBuffer<uint>       Buf              `#ifdef VB_SV0_TERM` ONLY --
//                                                                      the SDF edit list;
//                                                                      `sdf_field.hlsli`'s INCLUDE
//                                                                      CONTRACT requires it in
//                                                                      scope FIRST, and it keeps
//                                                                      `register(t0)` exactly as
//                                                                      `sdf_mesh_shadow.comp.hlsl`
//                                                                      spells it (DP6 design P1-4)
//   Set 2 (geometry, UNCHANGED) -- `vb_geom_fetch.hlsli`'s own `gMeshVerts[]`/`gMeshIndices[]`/
//   `gMeshMeta`.
//
// # The `VB_SV0_TERM` variant (VB-SV0 DP6b -- `docs/VB-SV0-DP6-DESIGN.md`, Decision 1)
//
// `-D VB_SV0_TERM=1` compiles the SDF-on-mesh shadow + contact-AO march INTO this pass, which
// already performs the per-covered-pixel `vb_geom_fetch` the march needs. That is DP6's whole
// consolidation: one producer instead of two (`sdf_mesh_shadow.comp.hlsl`, the dedicated pass, is
// retired at DP6e). It is a `-D` VARIANT and never an unconditionally-compiled runtime-gated span,
// because carrying the march dark measured **+10 128 B on a 15 888 B kernel = +64 %** instruction
// footprint at `13f1c9a3` (+75 % on `vb_resolve`) -- Decision 1's own number.
//
// EVERY addition sits inside `#ifdef VB_SV0_TERM`, and no local is introduced OUTSIDE a guard (the
// R9b hoisted-load lesson): with the flag undefined this file preprocesses CHARACTER-IDENTICAL to
// its pre-DP6b form, which is what keeps `vb_geo.comp.spv` / `vb_geo_mv.comp.spv` byte-frozen.
// `tests/vb_geo_preprocess_sync.rs` proves that two-sidedly against `git show <DP6b^>:`.
//
// The march span is the dedicated pass's, moved not rewritten (`sdf_mesh_shadow.comp.hlsl:142-183`):
// the shadow origin is lifted along the GEOMETRIC face normal (`vb_sv0_face_normal`, plan §4.2),
// the 5-tap AO runs along the SHADING normal, and `geo`/`n` are REUSED rather than re-derived.
// `#include "light_table.hlsli"` is ordered BEFORE `sdf_field.hlsli` exactly as the shipped
// consumer orders them (DP6 design P2-1).
//
// The store is GATED on `sv0_mode != 0u` (Decision 6 / P0-3) rather than unconditional as in the
// dedicated pass: the dedicated pass is only ever RECORDED when the mode is non-zero, so gating
// here is behaviourally identical on every `mode != 0` frame, and it makes the DP6d measurement
// arm B (`vb_sv0_host` true, mode 0 -- the variant bound with its write declared and skipped)
// buildable. The branch is a wave-uniform scalar header read: one compare, zero divergence.
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
//   (R9d MOTION variant, `vb_geo_mv` -- camera-reprojected static-geometry motion vectors; no
//   `rayQuery`, so the SAME `cs_6_0` target as the base compile suffices): add
//   `-T cs_6_0 -D MOTION=1 -Fo vb_geo_mv.comp.spv`
//   (DP6b SV0 variant, `vb_geo_sv0` -- the march compiled in; still no `rayQuery`, so `cs_6_0`
//   again): add `-T cs_6_0 -D VB_SV0_TERM=1 -Fo vb_geo_sv0.comp.spv`
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_geo.comp.spv

// VB-SV0 DP6b: `VB_SV0` unlocks `vb_geom_fetch.hlsli`'s `tri_p0/1/2` exports and
// `vb_sv0_face_normal`. It is DERIVED from the `-D VB_SV0_TERM=1` axis rather than spelled by the
// caller, so the command line carries ONE flag and the two names cannot drift apart -- `VB_SV0`
// itself is still never passed to dxc, which is the half of that header's definer contract this
// file keeps. The half it CHANGES is "zero new compile variants": this file is the contract's
// SECOND definer and does produce one (`vb_geo_sv0.comp.spv`). See
// `vb_geom_fetch.hlsli`'s `VB_SV0` paragraph, updated in the same rung -- the base/motion
// byte-identity now rests on this `#ifdef` guard, MEASURED by `tests/vb_geo_preprocess_sync.rs`,
// rather than on `vb_geo` not defining the macro at all.
#ifdef VB_SV0_TERM
#define VB_SV0
#endif

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

// binding 3: the Lighting-L0 light table.
//
// Bound-but-unread (the R2 contract) in the BASE and `-D MOTION=1` variants -- those write geometry
// only and do no shading. **The `-D VB_SV0_TERM=1` variant READS it** (VB-SV0 DP6b): the
// wave-uniform mode word via `load_vb_sdf_mesh_mode`, then the header + the primary directional's
// element for the march below. Still no shading -- the term is a visibility/occlusion scalar pair,
// not a lit colour -- but "unread" is false for one of the three modules and the distinction is
// observable: DXC strips a declared-but-unread `StructuredBuffer`, so `LightBuf` is ABSENT from the
// base/motion `.spv` interfaces and PRESENT in the SV0 one.
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
// R9d (`vb_geo_mv`, `-D MOTION=1`): the per-pixel camera-reprojected motion vector, WRITTEN by
// `main()` below (once per non-sentinel pixel).
[[vk::binding(1, 1)]] [[vk::image_format("rg16f")]] RWTexture2D<float2> gMotion : register(u9);

// R9d: the previous/current unjittered marcher-aligned view_proj pair the motion write
// reprojects against (camera-only, static-geometry motion -- the SDF leg's C6 semantics).
[[vk::binding(2, 1)]] cbuffer MotionCam : register(b10) {
    float4x4 mv_cur_view_proj;
    float4x4 mv_prev_view_proj;
};

// Marcher-aligned clip -> [0,1]^2 screen UV -- spliced VERBATIM from `deferred_pbr.hlsl`'s own
// `mv_clip_to_uv` (~430-432). The projection (`marcher_view_proj_rows`) bakes the y-flip into
// clip.y, so this is the plain NDC remap (NO extra negation) -- IDENTICAL to the gbuffer MV
// variant's `clip_to_uv`, so the mesh (raster) and VB (here) motion vectors land in ONE
// consistent UV space across the r1 ownership seam.
float2 mv_clip_to_uv(float4 clip) {
    return (clip.xy / clip.w) * 0.5 + 0.5;
}
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

#ifdef VB_SV0_TERM
// --- VB-SV0 DP6b: the term producer's own Set-1 surface + the march it hosts -----------------
//
// Set 1 @3: the R8G8_UNORM term target this variant OWNS -- R = soft-shadow visibility, G =
// contact AO, both in [0,1] with 1.0 = "no effect". The SAME image and the SAME semantics the
// dedicated pass wrote at its own `u6`; only the set/slot moved, because DP6 hosts the write in
// `vb_geo`'s aux set rather than in a layout of its own.
[[vk::binding(3, 1)]] [[vk::image_format("rg8")]] RWTexture2D<float2> gSdfTerm : register(u11);

// P2-1: the light-table decode comes FIRST -- `load_vb_sdf_mesh_mode` (the §3.1 word-7 bits 5..6
// mode read) and `load_light_header`/`load_light` are all reached through it, and the shipped
// consumer (`sdf_mesh_shadow.comp.hlsl:91`) orders it ahead of the field for the same reason.
#include "light_table.hlsli"

// Set 1 @4: the SDF edit list. `sdf_field.hlsli`'s INCLUDE CONTRACT requires `Buf` in scope FIRST.
// This variant is a strict FIELD-CONSUMER: it CALLS `field_distance` read-only and never edits;
// `field_distance` walks `min(Buf[0], MAX_SDF_EDITS)` edits -- already clamped.
//
// P1-4: `register(t0)` is KEPT, matching `sdf_mesh_shadow.comp.hlsl:96`'s own
// `vk::binding(10,0)` + `register(t0)` pairing. Only the Vulkan SLOT moved (Set-0 @10 -> Set-1 @4);
// the HLSL register is what `sdf_field.hlsli`'s contract names, and it is unchanged.
[[vk::binding(4, 1)]] StructuredBuffer<uint> Buf : register(t0);
#include "sdf_field.hlsli"

// The shadow-march tuning block -- a VERBATIM mirror of `sdf_mesh_shadow.comp.hlsl:104-112`,
// which itself mirrors `deferred_pbr.hlsl`'s, which mirrors the marcher's frozen A1 consts, so
// this march matches the one it replaces value-for-value. `sdf_shadow_leaves.hlsli`'s INCLUDE
// CONTRACT requires `MAX_IT`/`SHADOW_K`/`SHADOW_MINT`/`SHADOW_MINT_STEP`/`SHADOW_HIT_EPS` in
// scope; `GRAD_H`/`FIELD_LIPSCHITZ_L` come from `sdf_field.hlsli` above. `T_MAX` is the
// directional caster's march bound.
static const float EPS                = 0.001;
static const float T_MAX              = 10.0;
static const uint  MAX_IT             = 128u;
static const float SHADOW_K           = 8.0;
static const float SHADOW_MINT        = 16.0 * GRAD_H;
static const float SHADOW_MINT_STEP   = 16.0 * GRAD_H;
static const float SHADOW_HIT_EPS     = 2.0 * EPS;
static const float SHADOW_NDOTL_EPS   = 0.0;
static const float SHADOW_NORMAL_BIAS = 0.02; // normal-offset march-origin lift (anti grazing-acne)

#include "sdf_shadow_leaves.hlsli"
#endif

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

#if MOTION
    // R9d (`vb_geo_mv`): the per-pixel camera-reprojected motion vector, Delta-uv = prev - cur
    // (`shadow_temporal.comp.hlsl`'s documented "uv_prev - uv_cur" convention). Camera-only,
    // static-geometry motion (the SDF leg's C6 semantics; per-instance previous-transform
    // motion is a later rung). `geo.world_pos` is already reconstructed by the fetch above --
    // read directly here rather than introducing a new shared local, so the no-define compile's
    // existing statements stay byte-identical (the R9b hoisted-load lesson). Sentinel pixels
    // already returned above `vb_geom_fetch`, so every pixel reaching this point is real.
    gMotion[uint2(px, py)] =
          mv_clip_to_uv(mul(mv_prev_view_proj, float4(geo.world_pos, 1.0)))
        - mv_clip_to_uv(mul(mv_cur_view_proj, float4(geo.world_pos, 1.0)));
#endif

    PerInstanceMaterial pm = instance_materials[id.instance_id];
    MaterialGpu m = Materials[pm.id];
    float roughness = clamp(m.mrr.y, 0.045, 1.0); // fp32 floor, mirrors vb_resolve/the deferred resolve

    float2 oct = oct_encode(n);
    gThinNormal[uint2(px, py)] = float4(oct.x, oct.y, roughness, 1.0);

#ifdef VB_SV0_TERM
    // VB-SV0 DP6b: the term, computed in the host that already fetched the geometry. `geo` and
    // `n` above are REUSED, never re-derived -- that reuse is the whole consolidation win, and it
    // is also why no new local appears outside this guard.
    //
    // Hoisted ONCE per pixel -- a wave-uniform header read (plan §3.1, word 7 bits 5..6). The two
    // halves arm INDEPENDENTLY, so each block gates on its own bit: a shadow-only arm must not pay
    // the AO taps, and vice versa.
    uint sv0_mode = load_vb_sdf_mesh_mode(LightBuf);

    float vis = 1.0;
    if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u) {
        // The PRIMARY directional: the first `LIGHT_KIND_DIRECTIONAL` in `l0a` order -- the same
        // light the tails' `primary_dir_seen` latch selects, so the term this variant writes is
        // the term the dedicated pass wrote, for the same caster. Exactly ONE march per covered
        // pixel regardless of light count.
        LightHeader H = load_light_header(LightBuf);
        for (uint i = 0u; i < H.l0a_count; ++i) {
            LightElem L = load_light(LightBuf, i);
            if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
                float3 l = normalize(L.dir);
                float NoL = dot(n, l);
                // `NoL > SHADOW_NDOTL_EPS` stands in for the leaf's back-face early-out; the
                // RANGED leaf has none (its `n` parameter is unread -- the caller owns the
                // early-out). At `NoL <= 0` the tails multiply the direct term by `NoL` anyway,
                // so leaving `vis` at 1.0 is behaviourally identical and strictly cheaper.
                if (NoL > SHADOW_NDOTL_EPS) {
                    // March ORIGIN lifted along the GEOMETRIC face normal (plan §4.2), not the
                    // shading normal: from actual world positions it is the true plane normal
                    // under any affine instance transform, and it removes silhouette
                    // self-shadow acne.
                    float3 sv0_face_n = vb_sv0_face_normal(geo);
                    vis = sdf_soft_shadow_ranged(
                        geo.world_pos + sv0_face_n * SHADOW_NORMAL_BIAS, n, l, T_MAX);
                }
                break;
            }
        }
    }

    float ao = 1.0;
    if ((sv0_mode & VB_SDF_MESH_AO_BIT) != 0u) {
        // Contact AO: the 5-tap field-deficit AO along the SHADING normal. No origin bias: the
        // taps start at `h = AO_STEP`, already off-surface.
        ao = sdf_ao(geo.world_pos, n);
    }

    // P0-3: the store is GATED, so DP6d's arm B (host bound, mode 0) writes nothing while still
    // binding this module. Behaviourally identical to the dedicated pass on every `mode != 0`
    // frame -- that pass stores unconditionally but is only RECORDED when the mode is non-zero.
    if (sv0_mode != 0u) {
        gSdfTerm[uint2(px, py)] = float2(vis, ao);
    }
#endif
}
