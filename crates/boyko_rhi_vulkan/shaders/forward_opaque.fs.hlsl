// Multi-paradigm render-path plan, rung R4b: Forward render path v1 — the mesh raster
// FRAGMENT shader. Shades EVERY covered pixel inline against the full light table (all-lights,
// no froxel culling — Decision 2's shared post-geometry tail is not wired this rung; ForwardPlus
// / SDF-forward-march / VisibilityBuffer land later), sharing the SAME Cook-Torrance/GGX BSDF
// (`pbr_lighting.hlsli`, Decision 3) and the SAME combined CSM/punctual shadow visibility
// (`shadow_apply.hlsli`, Decision 7) the deferred resolve uses. NO `SV_Depth` write, NO
// `discard`, NO UAV — this fragment is a pure function of its rasterizer-interpolated inputs, so
// hardware early-Z (against the reverse-Z `depth` image `forward_opaque.vs.hlsl` writes) stays
// live (Decision 4).
//
// # v1 SCOPE CUT (orchestrator-directed)
//
//   * **Legs.** `Forward × Mesh` ONLY — the resolver (`boyko_render::render_path_config`)
//     collapses `Forward × {Both, Sdf}` to `Mesh` until R-SDFFWD lands (the existing
//     pre-SDF-forward-march ladder rule), so this shader is never asked to composite an SDF hit.
//   * **Lights.** ALL-LIGHTS on the base (this file's DEFAULT, `-D FROXEL` undefined) compile:
//     the point/spot loop reads the ENTIRE flat light table (`[0, l0a_count)` directionals+sky,
//     `[l0a_count, light_count)` point/spot). Rung R5 (ForwardPlus) added a SECOND compile of
//     this SAME source, `-D FROXEL=1` → `forward_opaque_froxel.fs.spv`: inside `#ifdef FROXEL`
//     the point/spot loop instead walks the pixel's froxel slice via `cluster_cull.hlsl`'s
//     `ClusterGrid`/`LightIndexList` (Set 0 bindings 5/6, froxel-compile-only — see that block's
//     doc), token-for-token the SAME lookup `deferred_pbr.hlsl`'s L1 block performs. The base
//     compile's tokens are byte-for-byte unperturbed by the `#else` arm, so `forward_opaque.fs.spv`
//     (plain `Forward`) stays byte-identical to its pre-R5 build.
//   * **Pre-light consumers.** NONE — no SSAO/DDGI/shadow-denoise/shadow-temporal. The resolver
//     (`cap_forward_v1_consumers`) forces every one of these OFF for a `Forward` boot with a warn
//     (`RenderPathDegrade::ForwardPreLightConsumersNotYetImplemented`), so `ao_final` below is a
//     constant `1.0` (no `gSsao`/thin-aux read) and no DDGI probe sample runs.
//   * **Motion / TAA.** NONE — no motion-vector MRT, no `MotionCam` UBO (the resolver forces TAA
//     off under Forward v1 too, `RenderPathDegrade::ForwardTaaNotYetImplemented`).
//   * **Material.** NON-TEXTURED ONLY: `base_color`/`metallic`/`roughness`/`emissive` come from
//     the `Materials` SSBO (`MaterialGpu`, byte-identical to the deferred resolve's binding 4),
//     keyed by the VS-forwarded flat `mat_id`. `#ifdef TEXTURED` is a seam for a later rung
//     (mirrors `gbuffer_mrt.fs.hlsl`'s own TEXTURED variant) — the bindless texture table is
//     therefore OMITTED entirely from this v1 pipeline (not merely unbound-but-declared): the
//     fragment shader references no `Texture2D[]`, so a textured variant lands as its OWN 3-set
//     pipeline (Set0/Set1-bindless/Set2-shadow) later, not by widening this one.
//   * **Shadows** (IN SCOPE): CSM cascades + the sparse spot/point atlas, sampled INLINE via
//     `shadow_apply.hlsli`'s `csm_visibility`/`spot_atlas_visibility`/`punctual_atlas_visibility`
//     — the SAME leaf functions (Decision 7) the deferred resolve calls, at Forward's OWN **Set 1**
//     bindings (renumbered from the plan's original Set 2 — see the Set-1 block's own doc for
//     why: with no bindless texture table this v1 rung, Set 1 is free, and a 2-set `[Set0, Set1]`
//     pipeline layout needs no empty-placeholder set to stay contiguous, unlike a 3-set
//     `[Set0, <empty Set1>, Set2]` shape, which a boot-time `create_bind_group_layout` invariant
//     rejects outright — a zero-binding layout is never valid). A DIFFERENT descriptor
//     set/binding layout than Deferred's single compute set, but the SAME global/type NAMES
//     `shadow_apply.hlsli`'s INCLUDE CONTRACT requires — see that header's doc for the "fixed
//     names, different binding numbers" idiom, mirroring `sdf_field.hlsli`'s `Buf` precondition.
//
// # Shadow baked-term simplification (mesh-only)
//
// Under `Deferred × Mesh`, the mesh raster's `gMaterial` MRT is UNCONDITIONALLY
// `float4(1,1,1,1)` (`gbuffer_mrt.fs.hlsl`) — i.e. `shadow = ao = 1.0` for every mesh pixel (no
// SDF analytic march exists to bake a self-shadow/AO term). Forward v1 is mesh-only by
// construction (the scope cut above), so its `vis`/`ao_final` starting points are the SAME
// implicit `1.0` — this shader hardcodes them rather than reading a nonexistent G-buffer lane,
// which is the ZERO-BEHAVIOR-CHANGE equivalent of Deferred's own mesh-pixel constants (verified
// against `deferred_pbr.hlsl`'s `vis = (punctual_shadow_mode != OFF) ? 1.0 : shadow` — both arms
// evaluate to `1.0` when `shadow == 1.0`, so Forward's `punctual_shadow`-only combine is the
// SAME expression, simplified).
//
// # Duplicated (not shared) pure helpers
//
// `env_brdf_approx`/`sun_kernel_exponent`/`sun_kernel`/`SUN_ENV_WEIGHT`/`SUN_KERNEL_EXPONENT_MIN`/
// `SUN_KERNEL_EXPONENT_MAX` are TOKEN-FOR-TOKEN copies of `deferred_pbr.hlsl`'s spans, NOT moved
// into `pbr_lighting.hlsli`: rung R4a already drew that header's boundary at "the DFG value
// (`dfg`) is a CALLER-computed parameter" (`eval_pbr_ambient_hemi`/`eval_pbr_sun_disc` both take
// `dfg` as an argument, never calling `env_brdf_approx` themselves) — widening that boundary is
// an architecture call this rung does not make; duplicating ~35 lines of pure, dependency-free
// math is the scoped alternative (mirrors this file's own `env_brdf_approx`-adjacent precedent:
// `csm_pcf_disc`/`atlas_pcf_disc` are already two near-identical siblings in `shadow_apply.hlsli`
// for an analogous "HLSL < 6.6 cannot pass texture/sampler handles" reason).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) TWICE from this ONE source
// (rung R5 / ForwardPlus, the fixed-priority-variant idiom):
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 forward_opaque.fs.hlsl -Fo forward_opaque.fs.spv
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main -D FROXEL=1 \
//       -fspv-target-env=vulkan1.3 forward_opaque.fs.hlsl -Fo forward_opaque_froxel.fs.spv
// The FIRST invocation (no `-D`) is the base `Forward` variant — byte-identical to its pre-R5
// build (verified `cmp`); the SECOND is the `ForwardPlus` froxel variant (`#ifdef FROXEL` below).

// `pbr_lighting.hlsli`'s INCLUDE CONTRACT precondition: `PI` + `LIGHT_UP` in scope first.
static const float PI = 3.14159265358979323846;
static const float3 LIGHT_UP = float3(0.0, 1.0, 0.0);

#include "pbr_lighting.hlsli"
#include "light_table.hlsli"

// --- Set 0 (Forward core, §G): Camera / light table / material SSBO -----------------------
//
// Bindings 0/1 of this SAME set are the VERTEX-only `instances`/`instance_materials` SSBOs
// (`forward_opaque.vs.hlsl`) — this fragment shader references neither (Vulkan descriptor sets
// are a per-stage UNION; a binding's HLSL declaration lives only in the stage(s) that read it).
// binding 2: the camera UBO — byte-identical shape to the deferred resolve's `Camera` (binding
// 5 there), reused for host-code economy even though this shader reads only `cam_eye`/
// `cam_forward`.
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
// binding 3: the Lighting-L0 light table (word-indexed `[LightHeaderGpu || GpuLight[]]`; decoded
// by `light_table.hlsli`) — the SAME SSBO the deferred resolve reads at its binding 6.
[[vk::binding(3, 0)]] StructuredBuffer<uint> LightBuf;
// binding 4: the material table — byte-identical `MaterialGpu` shape to the deferred resolve's
// binding 4 (`base_color`/`mrr`/`emissive`).
struct MaterialGpu {
    float4 base_color;
    float4 mrr;
    float4 emissive;
};
[[vk::binding(4, 0)]] StructuredBuffer<MaterialGpu> Materials;

// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the froxel cluster-grid pair, Set 0
// bindings 5/6 — compiled in ONLY for the `-D FROXEL=1` variant (`forward_opaque_froxel.fs.spv`);
// the base (Forward, this file's default compile) never declares them, so its Set 0 stays
// byte-identical at 5 bindings and `forward_opaque.fs.spv` stays byte-identical to the pre-R5
// build. Byte-identical shape to `deferred_pbr.hlsl`'s `ClusterGrid`/`LightIndexList` (bindings
// 8/9 there) — the L1 cluster-cull pass (`cluster_cull.hlsl`) writes both, reused verbatim; the
// lookup helpers (`load_cluster_params`/`cluster_xy_tile`/`cluster_z_slice`/
// `cluster_linear_index`) are `light_table.hlsli`'s shared L1 helpers, already `#include`d above.
#ifdef FROXEL
[[vk::binding(5, 0)]] StructuredBuffer<uint2> ClusterGrid;
[[vk::binding(6, 0)]] StructuredBuffer<uint> LightIndexList;
#endif

// --- Set 1 (Forward shadow, §G — RENUMBERED from Set 2, boot-panic fix): CSM + punctual atlas --
// --- `shadow_apply.hlsli`'s INCLUDE CONTRACT precondition (fixed names, Forward's OWN binding --
// --- numbers). Originally Set 2 with an empty Set 1 placeholder (Vulkan requires contiguous ---
// --- set indices 0..N); the placeholder's ZERO-BINDING layout violated
// `create_bind_group_layout`'s own `1..=MAX_BIND_GROUP_BINDINGS` invariant (a real boot panic in
// `GpuSceneBundles::boot`, caught in review). Renumbered to Set 1 so the pipeline layout is a
// plain 2-set `[Set0, Set1]` — no placeholder needed, matching the existing
// `create_graphics_pipeline_bindless` 2-set shape. Set 1 is therefore NOT the bindless texture
// table here (unlike Deferred/TEXTURED, where Set 1 IS the bindless set) — this v1 pipeline has
// no texture table at all (the file header's scope-cut note), so Set 1 is free for shadow. ---

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

// --- Duplicated pure helpers (see the file header's "Duplicated (not shared)" note) --------

// Karis "mobile" analytic environment BRDF approximation (no DFG LUT). VERBATIM copy of
// `deferred_pbr.hlsl::env_brdf_approx`.
float2 env_brdf_approx(float roughness, float NoV) {
    const float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
    const float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
    float4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * NoV)) * r.x + r.y;
    return float2(-1.04, 1.04) * a004 + r.zw;
}

// PBR P1 sun-disc kernel — VERBATIM copy of `deferred_pbr.hlsl`'s span (`sun_kernel_exponent` +
// `sun_kernel` + `SUN_ENV_WEIGHT`).
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

struct PsIn {
    float4 position  : SV_Position;
    float3 world_pos : WORLDPOS;
    float3 normal    : NORMAL;
    nointerpolation uint mat_id : MATID;
};

// NO `SV_Depth`: hardware reverse-Z from `forward_opaque.vs.hlsl`'s `SV_Position.z` stands
// unmodified -- early-Z stays live (Decision 4). Single render target (`lit`, COLOR).
float4 main(PsIn input) : SV_Target0 {
    float3 n = normalize(input.normal);
    float3 P = input.world_pos;
    float3 v = normalize(cam_eye.xyz - P);
    float NoV = max(dot(n, v), 1e-4);

    MaterialGpu m = Materials[input.mat_id];
    float3 base = m.base_color.rgb;
    float metallic = m.mrr.x;
    float roughness = clamp(m.mrr.y, 0.045, 1.0); // fp32 floor, mirrors the deferred resolve
    float reflectance = m.mrr.z;
    float3 emissive = m.emissive.rgb;
    float a = roughness * roughness; // GGX alpha = perceptual^2

    float3 f0 = lerp(0.16 * reflectance * reflectance, base, metallic);
    float3 diffuse_color = base * (1.0 - metallic);

    // PBR P0-D multi-scatter energy compensation -- ONE per-pixel term (view + roughness only),
    // hoisted before the light loop and reused at every specular site (direct + ambient), the
    // SAME `eval_pbr_*` contract `pbr_lighting.hlsli` documents.
    float2 dfg_v = env_brdf_approx(roughness, NoV);
    float  Ess = max(dfg_v.x + dfg_v.y, 1e-4);
    float3 energy_comp = 1.0 + f0 * (1.0 / Ess - 1.0);

    Surface surf;
    surf.n = n;
    surf.NoV = NoV;
    surf.a = a;
    surf.f0 = f0;
    surf.diffuse_color = diffuse_color;
    surf.energy_comp = energy_comp;

    // PBR P1: the REFLECTION vector, hoisted ONCE per pixel -- shared by the sky ambient
    // specular AND the per-directional HDR sun-disc term below.
    float3 R = reflect(-v, n);
    float hemi = dot(n, LIGHT_UP) * 0.5 + 0.5;

    // v1 scope cut: no SSAO/DDGI consumer is ever armed under Forward (the resolver caps them
    // off structurally) -- `ao_final` stays the Deferred-mesh-pixel constant `1.0` (see the file
    // header's "Shadow baked-term simplification" note). A future rung wiring SSAO under Forward
    // (a depth+normal prepass, Decision 8) changes ONLY this one local.
    float ao_final = 1.0;
    // PBR metal fix: decoupled specular occlusion (Filament SpecularAO_Lagarde), hoisted once
    // per pixel -- byte-identical formula to the deferred resolve's `spec_ao`.
    float spec_ao = saturate(pow(NoV + ao_final, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao_final);

    LightHeader H = load_light_header(LightBuf);
    uint csm_mode = load_csm_mode(LightBuf);
    uint punctual_shadow_mode = load_punctual_shadow_mode(LightBuf);

    float3 lit_direct = float3(0.0, 0.0, 0.0);
    float3 ambient = float3(0.0, 0.0, 0.0);
    bool primary_dir_seen = false;

    // L0a: the no-`P` front block (directionals + sky). ALL-LIGHTS -- no cluster/froxel lookup
    // (the plan's `#ifdef FROXEL` seam, left for R5/ForwardPlus).
    for (uint i = 0u; i < H.l0a_count; ++i) {
        LightElem L = load_light(LightBuf, i);
        if (light_kind(L) == LIGHT_KIND_DIRECTIONAL) {
            float3 l = normalize(L.dir);
            float NoL = max(dot(n, l), 0.0);
            // v1 mesh-only: no baked SDF shadow term (see the file header) -- `vis` starts at
            // the Deferred-mesh-pixel constant `1.0`, then CSM min-combines in.
            float vis = 1.0;
            if (!primary_dir_seen) {
                primary_dir_seen = true;
                if (csm_mode != CSM_MODE_OFF && NoL > 0.0) {
                    // The receiver's VIEW-SPACE depth for the cascade SELECT -- reconstructed
                    // directly from the rasterized world position (no ray-gen needed, unlike the
                    // deferred resolve's `view_t`-based reconstruction): `view_z = dot(cam_forward,
                    // P - cam_eye)`, the SAME quantity `csm_visibility`'s SELECT expects.
                    float view_z = dot(cam_forward.xyz, P - cam_eye.xyz);
                    vis = min(vis, csm_visibility(P, n, view_z, NoL));
                }
            }
            PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
            lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * vis) * L.color;

            // PBR P1 HDR sun disc -- the SAME `eval_pbr_sun_disc` (`pbr_lighting.hlsli`) the
            // deferred resolve calls, AO-gated via `spec_ao` (an environment term, not a
            // direct-light term).
            float sun_k = sun_kernel(R, l, a);
            float3 sun_spec_ambient = eval_pbr_sun_disc(surf, dfg_v, sun_k, L.color) * SUN_ENV_WEIGHT;
            ambient += sun_spec_ambient * spec_ao;
        } else if (light_kind(L) == LIGHT_KIND_SKY) {
            float3 sky_color = L.color;       // upper hemisphere
            float3 ground_color = L.pos;      // lower hemisphere (packed in the pos lane)
            ambient += eval_pbr_ambient_hemi(surf, R, dfg_v, sky_color, ground_color, hemi, ao_final, spec_ao);
        }
        // Point/spot (kinds 1/2) are the L0b block below, not this front block.
    }

    // L0b / L1: the point/spot block. Rung R5 (ForwardPlus, `#ifdef FROXEL`): map this pixel to
    // its froxel and walk ONLY the cluster's point/spot indices (`ClusterGrid`/`LightIndexList`,
    // token-for-token the SAME lookup `deferred_pbr.hlsl`'s L1 cluster block performs) instead of
    // the flat `[l0a_count, light_count)` scan — the `use_clusters` runtime gate (from the SAME
    // `cp.clusters_enabled` header lane the deferred resolve reads) preserves the L1 0%-gate: a
    // frame with no cull pass armed falls back to the IDENTICAL flat walk the base (non-FROXEL)
    // compile always runs. `view_z` is the SAME perspective view-space depth this file's own CSM
    // cascade-select already computes inline (Forward v1 is perspective-only, per
    // `forward_opaque.vs.hlsl`'s doc); `px`/`py`/`w`/`h` come from the rasterized fragment
    // coordinate + the Camera UBO's image dimensions (no compute-shader dispatch coordinate
    // exists in a raster fragment shader). The shared loop BODY below (BSDF combine, punctual
    // shadow, attenuation) is UNCHANGED between the two arms -- only the index-list SOURCE
    // differs, so a `-D FROXEL=1` recompile cannot perturb the flat-walk lighting math.
#ifdef FROXEL
    // Two DEFENCE terms beyond the enabled bit, bringing this site to the same three-term form
    // `vb_resolve.comp.hlsl`/`vb_shade.comp.hlsl` carry (it had NEITHER of them until now):
    //
    //   * non-zero dims (VB-P1b-0 C1). `cluster_z_slice` clamps to `(int)dim_z - 1`, which for
    //     `dim_z == 0` is `-1` and returns `0xFFFFFFFF`; `cluster_linear_index` then yields
    //     `0xFFFFFFFF` and `ClusterGrid[cluster]` reads gigabytes past the end. A zero-dims
    //     header is what `sync_cluster_light_gate` publishes on every boot whose
    //     `ResolvedRenderPath::froxel_light_cull` is false -- which today is EVERY ForwardPlus
    //     boot, this variant's own production path -- so the enabled bit alone was never a
    //     sufficient guard here.
    //   * CAPACITY (VB-P1k). `cluster_linear_index` is < dim_x*dim_y*dim_z by construction, so
    //     the LIVE header's dims are the only bound this read would otherwise have -- while
    //     `ClusterGrid` was SIZED at boot from `ClusterConfig::cluster_count()` and is never
    //     re-allocated, and `sync_cluster_light_gate` republishes the LIVE dims every frame. A
    //     post-boot `ClusterConfig` edit that GROWS the grid therefore makes this read leave the
    //     allocation, silently (`robustBufferAccess` is OFF, no GPU-assisted validation).
    //     `GetDimensions` reports the BOUND DESCRIPTOR's own element count (SPIR-V
    //     `OpArrayLength`) -- the allocation itself, not a host-side mirror of it -- so this term
    //     disarms the cluster walk for exactly the frames whose live grid does not fit the
    //     buffer, falling back to the in-bounds flat scan (which is also the CORRECT lighting for
    //     such a frame; clamping the index would silently shade against the wrong froxel).
    //
    // Both terms are inert on every armed, correctly-packed frame (dims nonzero together with the
    // enabled bit, and `cluster_count == grid_capacity` when boot dims == live dims -- every
    // shipping configuration), so ON==OFF equality is unaffected. Where no L1 cull is built,
    // `ClusterGrid` is bound to the light table as a placeholder (`targets.rs`); `GetDimensions`
    // then reports THAT buffer's element count, still a true bound on the bound descriptor -- and
    // the dims term has already disarmed the walk there anyway.
    ClusterParams cp = load_cluster_params(LightBuf);
    uint grid_capacity, grid_stride;
    ClusterGrid.GetDimensions(grid_capacity, grid_stride);
    uint cluster_count = cp.dim_x * cp.dim_y * cp.dim_z;
    bool use_clusters = (cp.clusters_enabled != 0u) && (cluster_count != 0u)
                     && (cluster_count <= grid_capacity);
    uint ps_count;   // number of point/spot lights to walk
    uint ps_offset;  // base into LightIndexList (clusters) or the flat block
    if (use_clusters) {
        float view_z = dot(cam_forward.xyz, P - cam_eye.xyz);
        uint px = (uint)input.position.x;
        uint py = (uint)input.position.y;
        uint2 tile = cluster_xy_tile(px, py, img_w_raw, img_h_raw, cp);
        uint zsl = cluster_z_slice(view_z, cp);
        uint cluster = cluster_linear_index(tile.x, tile.y, zsl, cp.dim_x, cp.dim_z);
        uint2 cell = ClusterGrid[cluster];
        ps_offset = cell.x;  // offset into LightIndexList
        ps_count = cell.y;   // count of indices in this froxel's slice
    } else {
        ps_offset = H.l0a_count;                  // flat block base
        ps_count = H.light_count - H.l0a_count;    // flat block length
    }
    for (uint jj = 0u; jj < ps_count; ++jj) {
        uint j = use_clusters ? LightIndexList[ps_offset + jj] : (ps_offset + jj);
#else
    for (uint j = H.l0a_count; j < H.light_count; ++j) {
#endif
        LightElem L = load_light(LightBuf, j);
        float3 toL = L.pos - P;
        float d2 = dot(toL, toL);
        float range2 = L.range * L.range;
        if (d2 > range2) {
            continue; // outside the cull sphere (range)
        }
        float inv_d = rsqrt(max(d2, 1e-8));
        float3 l = toL * inv_d;
        // Smooth windowed inverse-square falloff -- byte-identical to the deferred resolve's.
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

        // Shadow Phase 5 Inc-1/2: the punctual atlas hard-shadow term, via `shadow_apply.hlsli`.
        float punctual_shadow = 1.0;
        if (punctual_shadow_mode != PUNCTUAL_SHADOW_MODE_OFF) {
            uint slot = light_atlas_slot(L.kind);
            if (slot != SLOT_NONE) {
                float pnol = max(dot(n, l), 0.0);
                if (light_kind(L) == LIGHT_KIND_SPOT) {
                    punctual_shadow = spot_atlas_visibility(slot, P, n, pnol);
                } else if (light_kind(L) == LIGHT_KIND_POINT) {
                    punctual_shadow = punctual_atlas_visibility(slot, P, n, pnol);
                }
            }
        }

        float NoL = max(dot(n, l), 0.0);
        PbrDirectTerms bsdf = eval_pbr_direct_bsdf(surf, v, l, NoL);
        // v1 mesh-only: no baked SDF shadow term -- `vis` is implicitly `1.0` (see the file
        // header), so the combine is `NoL * punctual_shadow` (not `NoL * vis * punctual_shadow`).
        lit_direct += (bsdf.diffuse + bsdf.specular) * (NoL * punctual_shadow) * atten * L.color;
    }

    // Same tail as the deferred resolve: exposure LAST on the linear accumulator, THEN the
    // OWNER-SELECTED tonemap (`pbr_lighting.hlsli::tonemap_select`), THEN the manual gamma-2.2
    // OETF (see `pbr_lighting.hlsli`'s "PBR P0-C" doc for why the OETF is manual, not hardware).
    float3 lit = (lit_direct + ambient + emissive) * H.exposure;
    lit = tonemap_select(lit, load_tonemap_mode(LightBuf));
    lit = pow(lit, OETF_GAMMA_EXP);
    return float4(clamp(lit, 0.0, 1.0), 1.0);
}
