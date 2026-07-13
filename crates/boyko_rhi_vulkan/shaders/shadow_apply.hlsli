// Combined shadow-source apply (`shadow_apply.hlsli`) — Multi-Paradigm Render-Path
// Decision 7 (resolves W2), rung R4b textual extraction (O1/Decision 3 discipline).
//
// A VERBATIM cut of the CSM cascade + punctual (spot/point) atlas shadow-VISIBILITY
// leaf functions out of `deferred_pbr.hlsl`'s resolve: `csm_visibility` (the cascade
// SELECT + smooth cross-fade blend, CSM Inc 3), `spot_atlas_visibility` /
// `punctual_atlas_visibility` (Shadow Phase 5 Inc-1/Inc-2), and every helper they need
// (`shadow_grazing_scale`, the 13-tap PCF discs `csm_pcf_disc`/`atlas_pcf_disc`,
// `csm_sample_cascade`) plus their tuning consts (`CSM_NORMAL_BIAS`,
// `SPOT_SHADOW_NORMAL_BIAS`, `CSM_OVERLAP_PROPORTION`, `SHADOW_GRAZING_BIAS_MAX`).
// Each function/const body is copied CHARACTER-IDENTICAL to the span it was cut from;
// the only change is FILE-LEVEL ARRANGEMENT — the three tuning consts previously lived
// interleaved with their binding declarations at three DIFFERENT points in
// `deferred_pbr.hlsl` (adjacent to `CsmCascades`/`ShadowAtlas`), so consolidating them
// here (in front of the functions that read them — HLSL requires declaration before
// use, like C) necessarily reorders them RELATIVE TO EACH OTHER, but never edits a
// single token inside any one span. Mirrors the `pbr_lighting.hlsli` R0 rung precedent
// exactly (Decision 3's file doc): the authoritative gate for this rung is the image
// goldens (a moved textual span changes `__FILE__`/`__LINE__`, so DXC may legally emit
// non-identical SPIR-V for identical rendered output); SPIR-V byte-cmp is a secondary,
// best-effort check under `-Qstrip_debug`.
//
// This is the PERMANENT shadow-combination seam Decision 7 names: `forward_opaque.fs.hlsl`
// (R4b), `vb_resolve.hlsl`/`vb_shade.hlsl`, and `sdf_forward_march.hlsl`/`sdf_shade.hlsl`
// all `#include` this header and call `csm_visibility`/`spot_atlas_visibility`/
// `punctual_atlas_visibility` directly at their own shade site — exactly the same three
// leaf functions `deferred_pbr.hlsl`'s resolve calls inline today (that call-site
// COMBINATION logic — the per-light `min`/`mix` of CSM · punctual · SDF-soft · HWRT-vis
// into one `vis`, Decision 7's "shade-site variant binds the armed sources") stays
// call-site code (`deferred_pbr.hlsl`'s `main()` and, from R4b, `forward_opaque.fs.hlsl`'s
// `main()`), NOT folded into this header — it differs in shape between the directional
// and point/spot loop bodies and reads per-source resolve-local state (`csm_view_z`,
// per-light `pnol`, the punctual `slot`), so it is not a header-worthy pure function
// (mirrors `pbr_lighting.hlsli`'s R4a doc: "the shadow-visibility COMBINATION... stays in
// `deferred_pbr.hlsl`").
//
// # INCLUDE CONTRACT (precondition)
//
// The including TU (the shade-site shader) MUST declare, IN SCOPE BEFORE this header is
// `#include`d, byte-identical to `deferred_pbr.hlsl`'s own declarations (the binding
// NUMBERS may legitimately differ — Forward's Set-2 layout is not Deferred's Set-0 — but
// the TYPE NAMES / FIELD SHAPES referenced by the functions below must match exactly,
// the same "fixed register names both includers declare identically" idiom
// `sdf_field.hlsli`'s `Buf` precondition and `pbr_lighting.hlsli`'s `PI`/`LIGHT_UP`
// precondition use):
//
//   * `static const uint MAX_CASCADES` — the cascade-array capacity (`4` today).
//   * `struct CascadeData { float4x4 view_proj; float split_far; float texel_size;
//     float2 _pad; }` — MUST byte-mirror `boyko_render::CascadeData` (80 B).
//   * `Texture2DArray<float> gCsm` + `SamplerComparisonState gCsmCmp` — the cascade
//     depth-map array + its PCF comparison sampler, ONE combined descriptor (the
//     `register(tN)`/`register(sN)` SAME-NUMBER collapse idiom).
//   * a `cbuffer` exposing `CascadeData gCascades[MAX_CASCADES]` and `uint gCsmActive`
//     (the valid-cascade count) — the includer's own `CsmCascades`-shaped UBO.
//   * `static const uint M_SLOTS` — the atlas-array capacity (`16` today), MUST equal
//     `boyko_render::shadow_atlas::M_SLOTS`.
//   * `struct FaceTransform { float4x4 view_proj; float3 light_pos; float inv_range; }`
//     — MUST byte-mirror `boyko_render::FaceTransform` (80 B).
//   * `Texture2DArray<float> gShadowAtlas` + `SamplerComparisonState gShadowAtlasCmp` —
//     the spot/point atlas depth-map array + its PCF comparison sampler, ONE combined
//     descriptor.
//   * a `cbuffer` exposing `FaceTransform gFaces[M_SLOTS]` — the includer's own
//     `ShadowAtlas`-shaped UBO.
//
// `deferred_pbr.hlsl` satisfies this contract with its existing bindings 12/13/14/15
// (unedited by this extraction); `forward_opaque.fs.hlsl` (R4b) satisfies it with its own
// Set-2 CSM/atlas bindings, using the SAME type/global names against DIFFERENT register
// numbers — the functions below reference only the NAMES, never a literal `register(...)`.

// === CSM Rung-A normal-offset bias FACTOR (D6) — `csm_sample_cascade`'s acne guard ======
//
// The receiver lookup is pushed off the surface by `n * gCascades[0].texel_size *
// CSM_NORMAL_BIAS` so a grazing receiver does not self-shadow (acne). Kept LOW because
// the term is `min`-combined with the analytic SDF visibility — a slight acne is
// preferred over peter-panning (a too-large offset would lift the contact shadow off the
// floor and read as a floating caster). Owner-retunable; mirrors the host matrix golden's
// bias.
static const float CSM_NORMAL_BIAS = 2.0;

// === Shadow Phase 5 Inc-1-GPU normal-offset bias FACTOR — the spot/point acne guard =====
//
// The spot/point receiver lookup is pushed off the surface by `n * SPOT_SHADOW_NORMAL_BIAS`
// so a grazing receiver does not self-shadow (acne). A world-space constant (the spot map
// has no per-cascade `texel_size`); owner-retunable. Mirrors the host spot matrix golden's
// bias.
static const float SPOT_SHADOW_NORMAL_BIAS = 0.02;

// === CSM Increment 3 — Rung B cross-fade band WIDTH (D7) =================================
//
// As a PROPORTION of the SELECTED cascade's VIEW-Z range [prev_split, split_far]. Inside the
// trailing `overlap*range` slice the resolve ALSO samples cascade `c+1` and `mix`es the two
// visibilities so the cascade boundary is a smooth gradient instead of a hard resolution
// seam. No TAA on this engine => an ANALYTIC ramp, not a dither (a dither would shimmer
// without temporal accumulation). `0.2` = the band is the last 20% of each cascade — wide
// enough to hide the seam, narrow enough that the common pixel samples ONE cascade.
// Owner-retunable; mirrors the host `csm_select_blend` golden's constant.
static const float CSM_OVERLAP_PROPORTION = 0.2;

// === CSM Increment 1b/3 — the cascade shadow-map visibility sample (Rung B: N cascades) =======
//
// Projects the receiver world point `P` (normal-offset by `n` along `gCascades[c].texel_size *
// CSM_NORMAL_BIAS`, D6) into cascade `c`'s light-clip space, builds the shadow-map UV (Y-FLIPPED to
// match the engine's framebuffer convention — see below), and PCF-compares the receiver's
// light-space depth against the stored cascade depth via `gCsm.SampleCmpLevelZero(float3(uv, c))`.
// Returns the VISIBILITY in [0,1] (1 = lit, 0 = fully shadowed). One LAYER of the cascade array.
//
// UV Y-FLIP CONVENTION: the cascade depth pass renders with the SAME negative-viewport-free,
// Vulkan-default top-left framebuffer origin as the main raster pass; clip→NDC maps `clip.y` to
// the [-1,1] NDC Y, and the framebuffer's texel row 0 is NDC Y = -1's projection AFTER the
// Vulkan Y-down convention. The engine's other reprojection (`project_to_screen`, the SSCS inverse)
// applies a `(-ndc_y) * 0.5 + 0.5` flip to convert NDC→UV; this CSM lookup applies the IDENTICAL
// flip (`uv.y = 1 - (clip.y/clip.w * 0.5 + 0.5)`) so the cascade UV addresses the same texel the
// depth pass wrote. (The ortho light projection has `clip.w == 1`, so the perspective divide is a
// no-op, but it is kept for generality.)
//
// O1 MAJORNESS: `gCascades[c].view_proj` is the SAME column-major matrix the depth VS pushed at
// `@0` for cascade `c`, so `mul(view_proj, float4(P_off,1))` here reprojects EXACTLY as the depth
// VS projected the caster — the host matrix golden (compute.rs) pins this agreement.
// Slope-scaled shadow normal-offset multiplier. A near-GRAZING receiver (small NoL — a vertical
// face under a steep light) needs a LARGER along-normal offset to clear the per-texel light-space
// depth slope, the source of self-shadow ACNE (the dark band on the column / the diagonal wedge on
// the CSM caster box). A head-on receiver (NoL ~ 1) keeps the minimal offset so contact shadows do
// not PETER-PAN (light leak at the base). `1/NoL` is the standard slope term, floored at the light
// horizon and capped so a silhouette pixel cannot offset unboundedly. Shared by CSM + spot + point.
static const float SHADOW_GRAZING_BIAS_MAX = 6.0;
float shadow_grazing_scale(float nol) {
    return clamp(1.0 / max(nol, 1.0e-3), 1.0, SHADOW_GRAZING_BIAS_MAX);
}

// === Shadow-edge PCF (anti-scintillation) =======================================================
//
// A single-tap shadow-map compare leaves the shadow boundary a 1-2 screen-pixel STEP that
// requantizes under sub-pixel camera motion: the edge pixels flip 0<->1 every frame while the
// camera moves, so the (world-fixed!) shadow visibly "dances" in motion and is rock-stable when
// stopped. Proven by the shadow-motion A/B harness (`shadow_motion_ab_dump`): the frame is a pure
// function of the camera pose (no cross-frame race), and a 3 mrad yaw flips shadow-edge pixels at
// near-full swing (max channel delta 226/255).
//
// The fix is SPATIAL, not temporal (this engine deliberately has NO TAA — the analytic-ramp
// convention, see CSM_OVERLAP_PROPORTION): widen the binary edge into a ~4-texel tent ramp so
// sub-pixel motion produces proportional visibility deltas instead of full flips.
//
// 13-tap TENT DISC over the hardware 2x2 comparison taps, all with COMPILE-TIME texel offsets
// (the `int2` offset overload — SPIR-V ConstOffset caps offsets at [-8, 7]; no dimension query,
// no per-tap UV math; offsets clamp at the map edge per the sampler address mode). Taps: center
// (w 4), the ±2 ring of 8 (w 2), the ±4 axis ring of 4 (w 1) — sum 24. With the hardware 2x2
// bilinear under each tap the kernel integrates a smooth ~10-texel footprint (2048-map texel =
// 0.0078 wu ⇒ ~0.08 wu penumbra ≈ 2-3 screen px at room viewing distance — wide enough that a
// 1-2 px/frame camera drift moves the edge by a FRACTION of its ramp, killing the crawl, while
// the sun shadow still reads crisp). The A/B harness verified the 3x3 (1-px ramp) variant was
// NOT wide enough: shadow-edge flip counts barely moved; ramp width must exceed the per-frame
// image drift by 2-3x.
//
// The tap pattern is FIXED (no per-pixel rotation/noise): screen-anchored noise would reintroduce
// exactly the temporal boil this kernel removes (the no-TAA analytic-ramp convention again).
// Cost: +12 comparison taps per shadowed sample, only inside the csm_mode / shadow_mode
// structural gates (the 0%-gate scenes never run any of this).
//
// Two sibling helpers (not one) because HLSL < 6.6 cannot pass texture/sampler objects as
// arguments portably; each hardcodes its own combined-descriptor pair.

float csm_pcf_disc(float2 uv, float layer, float ref) {
    float3 c = float3(uv, layer);
    float v;
    v  = gCsm.SampleCmpLevelZero(gCsmCmp, c, ref) * (4.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2(-2,  0)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 2,  0)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 0, -2)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 0,  2)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2(-2, -2)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 2, -2)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2(-2,  2)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 2,  2)) * (2.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2(-4,  0)) * (1.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 4,  0)) * (1.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 0, -4)) * (1.0 / 24.0);
    v += gCsm.SampleCmpLevelZero(gCsmCmp, c, ref, int2( 0,  4)) * (1.0 / 24.0);
    return v;
}

float atlas_pcf_disc(float2 uv, float layer, float ref) {
    float3 c = float3(uv, layer);
    float v;
    v  = gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref) * (4.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2(-2,  0)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 2,  0)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 0, -2)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 0,  2)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2(-2, -2)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 2, -2)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2(-2,  2)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 2,  2)) * (2.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2(-4,  0)) * (1.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 4,  0)) * (1.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 0, -4)) * (1.0 / 24.0);
    v += gShadowAtlas.SampleCmpLevelZero(gShadowAtlasCmp, c, ref, int2( 0,  4)) * (1.0 / 24.0);
    return v;
}

float csm_sample_cascade(uint c, float3 P, float3 n, float nol) {
    float3 P_off = P + n * (gCascades[c].texel_size * CSM_NORMAL_BIAS * shadow_grazing_scale(nol));
    float4 clip = mul(gCascades[c].view_proj, float4(P_off, 1.0));
    if (clip.w <= 0.0) {
        return 1.0;                        // behind the light plane — treat as lit (no shadow data)
    }
    float3 ndc = clip.xyz / clip.w;
    float2 uv;
    uv.x = ndc.x * 0.5 + 0.5;
    // NO second Y-flip: the cascade depth pass renders into a POSITIVE-height viewport (it does NOT
    // use a negative-height Vulkan flip), so the hardware stores the occluder at fy=(ndc.y*0.5+0.5)*DIM
    // — and `csm_cascade_view_proj` ALREADY negates light-up once (its `-inv_h*up` clip row). A second
    // `1.0 - (...)` here would Y-flip the READ vs the WRITE, mirroring every shadow across the cascade's
    // light-up=0 line (invisible only when the caster sits on that line — the camera-fit fixed point;
    // a world-fixed off-axis caster shows the full mirror). Match the write convention exactly.
    uv.y = ndc.y * 0.5 + 0.5;
    // Outside this cascade's footprint there is no shadow data for it — treat as lit (the SELECT
    // already picked the tightest in-range cascade; a footprint miss here means fully lit). `ref`
    // is the receiver's light-space NDC depth (Vulkan [0,1] depth range).
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
        return 1.0;
    }
    float ref = ndc.z;
    // PCF: 13-tap tent disc over the hardware 2x2 comparisons (LessOrEqual) — the tent-weighted
    // lit fraction of a ~10-texel footprint at array layer `c` (anti-scintillation, see
    // `csm_pcf_disc`).
    return csm_pcf_disc(uv, (float)c, ref);
}

// === CSM Increment 3 — Rung B: the cascade SELECT + smooth cross-fade band (D7) ===============
//
// SELECT (the interval compare-chain): `view_z` is the receiver's VIEW-SPACE depth (`dot(P -
// cam_eye, cam_forward)` for PERSP, `view_t` for ORTHO — the SAME quantity the L1 froxel slice
// uses), and the PSSM `split_far` boundaries are ALSO view-space (the resolve fits them that way),
// so the SELECT runs in VIEW-Z LINEAR space (the critic's open Q4 answer). The chosen cascade is
// the FIRST `c` whose `view_z < gCascades[c].split_far` — i.e. the tightest cascade still covering
// the pixel. Past the LAST active split → no cascade covers the pixel → fully lit (return 1).
//
// The chain is BRANCH-LIGHT (Principle 1, this is a hot compute path): the selected index is the
// COUNT of splits `view_z` has already passed — `sel = sum_c step(split_far[c], view_z)` over a
// single bounded loop with uniform control flow (no per-lane early `return`; every lane walks the
// same `gCsmActive` iterations). `sel == gCsmActive` ⇔ past every split ⇔ uncovered (fully lit).
//
// BLEND (the analytic cross-fade): inside the trailing `CSM_OVERLAP_PROPORTION * range` slice of
// the selected cascade's view-z range `[prev_split, split_far]`, ALSO sample cascade `sel+1` (when
// it exists) and `lerp` the two visibilities, `band_t` ramping 0→1 across the band. The COMMON case
// (outside the band, or the last cascade) samples ONE cascade — `band_t == 0` so the second sample
// is multiplied out (`lerp(a, b, 0) == a`); the `sel+1` sample is taken unconditionally inside the
// `csm_mode` block but is cheap and never read when `band_t == 0`. Blend space: VIEW-Z LINEAR
// (matching `split_far`), so the seam fades over a constant-depth slice.
//
// Returns the blended VISIBILITY in [0,1]. Host mirror: `csm_host_select_blend` (the demo test).
float csm_visibility(float3 P, float3 n, float view_z, float nol) {
    if (gCsmActive == 0u) {
        return 1.0;                        // no cascades fitted — fully lit (defensive; gated above)
    }
    // SELECT: the selected cascade index = the number of splits the pixel has passed. `prev_split`
    // tracks the near edge of the selected cascade (the previous cascade's far, 0 for cascade 0).
    uint sel = 0u;
    float prev_split = 0.0;
    for (uint c = 0u; c < gCsmActive; ++c) {
        float far_c = gCascades[c].split_far;
        float passed = step(far_c, view_z);  // 1 when view_z >= this split (the pixel is beyond it)
        prev_split = prev_split + passed * (far_c - prev_split); // latch the near edge as splits pass
        sel += (uint)passed;
    }
    // Past the last active split (`sel == gCsmActive`): no cascade covers this pixel → fully lit (no
    // shadow data beyond the shadow distance).
    if (sel >= gCsmActive) {
        return 1.0;
    }

    float vis_sel = csm_sample_cascade(sel, P, n, nol);

    // BLEND band: the trailing `overlap * range` of the selected cascade's view-z range. Outside
    // the band `band_t == 0` (one-cascade common case); inside it ramps 0→1 to `sel + 1`.
    float far_sel = gCascades[sel].split_far;
    float range = max(far_sel - prev_split, 1.0e-4);      // guard a degenerate (zero-width) cascade
    float band_start = far_sel - CSM_OVERLAP_PROPORTION * range;
    float band_t = saturate((view_z - band_start) / max(far_sel - band_start, 1.0e-4));
    // Only blend toward a NEXT cascade that exists; the last cascade has no successor → no fade-out
    // (its far edge is the shadow distance, beyond which `sel >= gCsmActive` already returned lit).
    float has_next = (sel + 1u < gCsmActive) ? 1.0 : 0.0;
    band_t *= has_next;
    uint next = min(sel + 1u, gCsmActive - 1u);            // clamp the index (multiplied out if !has_next)
    float vis_next = csm_sample_cascade(next, P, n, nol);
    return lerp(vis_sel, vis_next, band_t);
}

// === Shadow Phase 5 Inc-1-GPU — the SPOT atlas shadow-map visibility sample =====================
//
// Projects the receiver world point `P` (normal-offset by `n * SPOT_SHADOW_NORMAL_BIAS`, the acne
// guard) into atlas slot `s`'s light-clip space, builds the shadow-map UV (Y-FLIPPED to match the
// engine's framebuffer convention — IDENTICAL to `csm_sample_cascade`), and PCF-compares the
// receiver's light-space depth against the stored spot depth via
// `gShadowAtlas.SampleCmpLevelZero(float3(uv, s))`. Returns the VISIBILITY in [0,1] (1 = lit, 0 =
// fully shadowed). One LAYER of the atlas array.
//
// O1 MAJORNESS: `gFaces[s].view_proj` is the SAME column-major matrix the depth pass pushed at `@0`
// for slot `s`, so `mul(view_proj, float4(P_off,1))` here reprojects EXACTLY as the depth pass
// projected the caster — the host spot matrix golden pins this agreement.
//
// SPOT (Inc 1) uses the perspective NDC-z directly; POINT (Inc 2) will branch on `gFaces[s].inv_range`
// + `light_pos`, not added here (the spot-only increment).
float spot_atlas_visibility(uint s, float3 P, float3 n, float nol) {
    float3 P_off = P + n * (SPOT_SHADOW_NORMAL_BIAS * shadow_grazing_scale(nol));
    float4 clip = mul(gFaces[s].view_proj, float4(P_off, 1.0));
    if (clip.w <= 0.0) {
        return 1.0;                        // behind the light plane — treat as lit (no shadow data)
    }
    float3 ndc = clip.xyz / clip.w;
    float2 uv;
    uv.x = ndc.x * 0.5 + 0.5;
    uv.y = ndc.y * 0.5 + 0.5;              // NO 2nd Y-flip (same as csm_sample_cascade): the spot matrix
                                           // already Y-flips once (-f*up) into a positive-height viewport;
                                           // a 1.0-(...) double-flips = latent mirror (masked when the
                                           // caster sits on the cone axis — the fixed point).
    // Outside this spot's cone footprint there is no shadow data — treat as lit (the cone falloff
    // already drove the contribution to 0 at the edge). `ref` is the receiver's light-space NDC
    // depth (Vulkan [0,1] depth range).
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
        return 1.0;
    }
    float ref = ndc.z;
    // PCF: 13-tap tent disc over the hardware 2x2 comparisons (LessOrEqual) — the tent-weighted
    // lit fraction of a ~10-texel footprint at array layer `s` (anti-scintillation, see
    // `atlas_pcf_disc`).
    return atlas_pcf_disc(uv, (float)s, ref);
}

// === Shadow Phase 5 Inc-2 (POINT cube) — the OMNI point atlas shadow-map visibility sample =======
//
// A POINT light occupies SIX CONTIGUOUS atlas layers `base..base+6` (the ±X/±Y/±Z cube faces, the
// host fit's `[+X, -X, +Y, -Y, +Z, -Z]` order). All six faces share the SAME `light_pos`/`inv_range`
// (read from `gFaces[base]`), and the depth pass stored, on each face, the LINEAR RADIAL distance
// `saturate(length(world - light_pos) * inv_range)` (`punctual_depth.fs`). So the resolve:
//   1. forms `dir = P - light_pos` (light -> receiver),
//   2. MAJOR-AXIS face-selects: the face whose axis has the largest |component| of `dir`
//      (branchless `step`/`abs` 6-way pick) — `face` in `[0,6)` matching the host order,
//   3. builds the per-face UV via the standard cube-map `(major, sc, tc)` mapping (the two minor
//      axes divided by |major|, then `*0.5 + 0.5`, with the per-face sign/swizzle convention that
//      matches the host look-at basis so the lookup hits the texel the depth pass wrote),
//   4. compares the receiver's OWN normalized radial distance `ref = length(dir) * inv_range`
//      against the stored face distance via `SampleCmpLevelZero` (LessOrEqual — same sense as the
//      spot path: a receiver farther than the stored occluder is shadowed).
// Returns the VISIBILITY in [0,1] (1 = lit, 0 = fully shadowed).
//
// The UV convention here is pinned to the host `point_faces` look-at (right-handed,
// `Affine3A::look_at_rh(eye, eye + axis, +Y)`), the SAME `point_host_project` mirror the matrix
// golden asserts. A normal-offset bias (`P + n * SPOT_SHADOW_NORMAL_BIAS`) on the distance origin
// guards grazing self-shadow acne, exactly like the spot path.
float punctual_atlas_visibility(uint base, float3 P, float3 n, float nol) {
    float3 P_off = P + n * (SPOT_SHADOW_NORMAL_BIAS * shadow_grazing_scale(nol));
    float3 light_pos = gFaces[base].light_pos;
    float inv_range = gFaces[base].inv_range;
    float3 dir = P_off - light_pos;                       // light -> receiver
    float3 a = abs(dir);

    // Major-axis face select (branchless). face order: +X=0,-X=1,+Y=2,-Y=3,+Z=4,-Z=5.
    // `ma` is the magnitude of the dominant axis; `uvc = (right.d, -(up.d))` the two minor coords
    // (sc, tc) for that face's basis. The per-face right/up come from the host fit's RH look-at
    // (`spot_demo_view_proj` basis: right = norm(cross(up_hint, fwd)), up = cross(fwd, right), up_hint
    // = +Y except +Z for a ±Y axis). The depth pass projects `ndc.x = right.d / fwd.d`,
    // `ndc.y = -(up.d) / fwd.d`, so `uvc / ma` reproduces NDC EXACTLY (`fwd.d == ma` on each face).
    // NOTE: this hand-coded reconstruction DROPS the perspective `f = cot(FOV/2)` factor, valid ONLY
    // because cube faces are 90° (`f == 1`). The Rust bake pins that with a compile-time assert on
    // `POINT_FACE_FOV_Y == π/2` (shadow_atlas.rs); if that FOV ever changes, sample the uploaded
    // per-face `view_proj` here (like the spot path) instead of this table.
    uint face;
    float ma;
    float2 uvc;
    if (a.x >= a.y && a.x >= a.z) {
        ma = a.x;
        face = (dir.x >= 0.0) ? 0u : 1u;
        // +X: right = -Z, up = +Y => sc = -z, tc = -y.   -X: right = +Z, up = +Y => sc = z, tc = -y.
        uvc = (dir.x >= 0.0) ? float2(-dir.z, -dir.y) : float2(dir.z, -dir.y);
    } else if (a.y >= a.x && a.y >= a.z) {
        ma = a.y;
        face = (dir.y >= 0.0) ? 2u : 3u;
        // +Y: right = -X, up = +Z => sc = -x, tc = -z.   -Y: right = +X, up = +Z => sc = x, tc = -z.
        uvc = (dir.y >= 0.0) ? float2(-dir.x, -dir.z) : float2(dir.x, -dir.z);
    } else {
        ma = a.z;
        face = (dir.z >= 0.0) ? 4u : 5u;
        // +Z: right = +X, up = +Y => sc = x, tc = -y.    -Z: right = -X, up = +Y => sc = -x, tc = -y.
        uvc = (dir.z >= 0.0) ? float2(dir.x, -dir.y) : float2(-dir.x, -dir.y);
    }
    // Project the minor coords onto the face plane (divide by |major|), then map [-1,1] -> [0,1].
    // The Y axis is FLIPPED to match the engine's framebuffer convention (the depth pass rendered
    // with the same `view_proj` Y-flip the spot/cascade paths use).
    float inv_ma = (ma > 1e-8) ? (1.0 / ma) : 0.0;
    float2 uv;
    uv.x = uvc.x * inv_ma * 0.5 + 0.5;
    // NO second Y-flip (the CSM mirror, applied to the point cube). uvc.y is ALREADY -(up.dir) — the
    // face matrix's own `-f*up` Y-flip — and the atlas depth pass writes into a POSITIVE-height
    // viewport (no negative-height Vulkan flip), so the stored texel is at ndc.y*0.5+0.5. A `1.0 - (...)`
    // here would Y-mirror every point shadow about uv.y=0.5 (invisible only for a caster on the face's
    // central axis — the fixed point; an off-axis box/slab shows the full mirror). Net Y inversions = 1.
    uv.y = uvc.y * inv_ma * 0.5 + 0.5;
    // The receiver's own normalized radial distance — the SAME expression the depth FS stored, so
    // the LessOrEqual compare is apples-to-apples. Saturated to the [0,1] depth range.
    float ref = saturate(length(dir) * inv_range);
    uint layer = base + face;
    // PCF: 13-tap tent disc (see `atlas_pcf_disc`). Taps that cross a cube-face UV edge clamp to
    // the face border texel; the stored value is the RADIAL distance (continuous across faces), so
    // the clamped tap reads a near-correct neighbor — an acceptable few-texel seam approximation.
    return atlas_pcf_disc(uv, (float)layer, ref);
}
