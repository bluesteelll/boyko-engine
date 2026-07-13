// Shared PBR BRDF terms (`pbr_lighting.hlsli`) — Multi-Paradigm Render-Path Decision 3.
//
// Rung-0 (textual extraction, O1/Decision 3): a VERBATIM cut of the Cook-Torrance/GGX
// primitive terms + the shadow-safe normalize out of the deferred resolve
// (`deferred_pbr.hlsl`). Nothing here was reordered or edited — the source is
// character-identical to the region it was cut from, so DXC emits (best-effort)
// byte-identical SPIR-V and the host golden stays byte-exact. The authoritative gate
// for this rung is the image goldens (a moved textual span changes `__FILE__`/
// `__LINE__`, so dxc MAY legally emit non-identical SPIR-V for identical rendered
// output); SPIR-V byte-cmp is a secondary, best-effort check under `-Qstrip_debug`.
//
// This is the PERMANENT BRDF seam (Decision 3): later rungs `#include` this header
// from `forward_opaque.fs.hlsl`, `vb_resolve.hlsl`/`vb_shade.hlsl`, and
// `sdf_forward_march.hlsl`/`sdf_shade.hlsl` so Cook-Torrance/GGX exists exactly once
// across every render path. Rung-0 extracted only the primitives that are PURE
// functions of their parameters (no G-buffer/cbuffer reads) and explicitly deferred
// the `eval_pbr_direct`/`eval_pbr_ambient` public surface (§C) "to a later rung once
// the other paths need it".
//
// Rung-4a (this deepening) is that rung's FIRST half: it factors the parts of
// `deferred_pbr.hlsl`'s light loop that are STILL pure functions of already-hoisted
// per-pixel values — the `Surface` carrier, the D/V/F/spec/diff Cook-Torrance
// combination (`eval_pbr_direct_bsdf`, byte-for-byte duplicated between the
// directional and point/spot loop bodies before this rung), and the sky-hemisphere /
// sun-disc ambient terms (`eval_pbr_ambient_hemi` / `eval_pbr_sun_disc`). The
// shadow-visibility COMBINATION (Decision 7 / `shadow_apply.hlsli`) and the
// per-light attenuation/terminator-wrap modulation stay in `deferred_pbr.hlsl`: they
// read per-source textures/TLAS/SSBOs (CSM/atlas/HWRT — resolve-specific, not pure)
// and differ in shape between the directional and punctual light sites, so folding
// them in now would force an artificial reassociation of the existing multiply
// chains for zero behavioral gain. R4b (the Forward FS) is expected to show the
// REAL cross-path shape that outer modulation needs; only then does widening this
// surface stop being a guess.
//
// Rung-4b (Forward render path R4b, alongside the `shadow_apply.hlsli` extraction) adds a
// SECOND, independent relocation from the SAME resolve span: the output-stage tonemap
// operators (`aces_fitted`/`khronos_pbr_neutral`/`reinhard_jodie`/`tonemap_select` + their
// `ACES_IN`/`ACES_OUT`/`OETF_GAMMA_EXP` constants) — pure functions of `(color, mode)` with
// no texture/buffer reads beyond the `TONEMAP_*` mode constants (`light_table.hlsli`, now
// `#include`d directly by this header — see below), so every render path applies the OWNER-
// SELECTED tonemap curve identically, not a hand-copied duplicate. TOKEN-FOR-TOKEN cut from
// `deferred_pbr.hlsl`'s "PBR P0-C" span, same Decision-3 image-golden-authoritative gate as
// every other extraction in this file.
//
// # INCLUDE CONTRACT (precondition)
//
// `static const float PI` MUST be declared in the including TU BEFORE this header is
// `#include`d — `D_GGX` below reads it. `static const float3 LIGHT_UP` MUST also be
// declared before the `#include` — `eval_pbr_ambient_hemi` below reads it. The
// including TU owns both constants; this header references them (the same pattern
// `sdf_field.hlsli` uses for its `Buf` precondition). This header itself `#include`s
// `light_table.hlsli` (guarded, safe to double-include) for the `TONEMAP_*` mode
// constants `tonemap_select` switches on — the including TU does NOT need to include it
// separately first.

#include "light_table.hlsli"

// --- Cook-Torrance / GGX terms (Filament real-time forms) -----------------------------

// GGX/Trowbridge-Reitz normal distribution. `a` is the remapped roughness (perceptual^2).
float D_GGX(float NoH, float a) {
    float a2 = a * a;
    float d = (NoH * a2 - NoH) * NoH + 1.0;     // = (NoH^2)(a2-1)+1, the stable rearrange
    return a2 / (PI * d * d);
}

// Height-correlated Smith visibility (folds the 1/(4 NoL NoV) of the specular denominator).
float V_SmithGGXCorrelated(float NoV, float NoL, float a) {
    float a2 = a * a;
    float lambdaV = NoL * sqrt((NoV - a2 * NoV) * NoV + a2);
    float lambdaL = NoV * sqrt((NoL - a2 * NoL) * NoL + a2);
    return 0.5 / max(lambdaV + lambdaL, 1e-5);
}

// Schlick Fresnel.
float3 F_Schlick(float u, float3 f0) {
    float f = pow(1.0 - u, 5.0);
    return f0 + (1.0 - f0) * f;
}

// Zero-/non-finite-guarded normalize — the FAITHFUL mirror of the host oracle's
// `boyko_sdf_math::v_normalize` (compute.rs reuses it for every golden lighting
// normalize). HLSL's intrinsic `normalize(0)` is `0/0 == NaN`, whereas the host
// returns `float3(0,0,0)`; that divergence is the L1 black-pixel bug. At a surface
// whose normal faces AWAY from a still-in-range point/spot light the half-vector
// `v + l` can be ~zero (the light direction `l` is ~opposite the view dir `v`):
// the host's `v_normalize(v+l)` yields `[0,0,0]` -> NoH = LoH = 0 -> a FINITE spec
// term that the `NoL == 0` factor then zeroes, while the GPU's `normalize(v+l)`
// yields NaN -> NaN spec -> `NaN * 0 == NaN` -> `pack_unorm(NaN) == 0` -> a pure
// BLACK pixel. Using this guard for every per-light `normalize` restores bit-parity
// with the host (the guard is byte-identical to `normalize` on all non-degenerate
// inputs, so the L0a/L0b/L1-off paths that already match are unchanged).
float3 safe_normalize(float3 a) {
    float len = sqrt(dot(a, a));
    // FLT_MIN floor + isfinite guard, matching v_normalize's
    // `len <= f32::MIN_POSITIVE || !len.is_finite()` degenerate branch.
    if (len <= 1.17549435e-38 || !isfinite(len)) {
        return float3(0.0, 0.0, 0.0);
    }
    return a / len;
}

// --- Rung-4a public surface: the per-pixel Surface carrier + the shared direct/ambient
// evaluations (see the file header for exactly what stays in `deferred_pbr.hlsl` and why) ----

// Surface — the per-pixel PBR parameters `deferred_pbr.hlsl::main()` hoists ONCE
// (view/roughness/metallic-derived), shared by every light sample AND the ambient
// terms below. Field order matches the hoist order in `main()`.
struct Surface {
    float3 n;              // world normal
    float  NoV;             // max(dot(n, V), eps) — V is pixel-constant, hoisted once
    float  a;                // GGX alpha = roughness^2
    float3 f0;                 // dielectric/metal Fresnel reflectance at normal incidence
    float3 diffuse_color;       // albedo * (1 - metallic)
    float3 energy_comp;          // PBR P0-D multi-scatter energy compensation (view+roughness only)
};

// The (diffuse, specular) pair `eval_pbr_direct_bsdf` returns. The caller applies its
// own NoL / shadow-visibility / attenuation / terminator-wrap modulation — see the
// file header for why that stays at the call site.
struct PbrDirectTerms {
    float3 diffuse;
    float3 specular;
};

// eval_pbr_direct_bsdf — the Cook-Torrance direct-light BSDF combination: D_GGX *
// V_SmithGGXCorrelated * F_Schlick specular (energy-compensated) + Lambert diffuse.
// TOKEN-FOR-TOKEN identical to the block `deferred_pbr.hlsl`'s directional and
// point/spot light-loop bodies each duplicated inline before this rung — a pure
// function of its parameters (no texture/buffer reads), so Forward FS / VB shade /
// SDF forward shade share this EXACT evaluation (Decision 3's single-BRDF-source
// requirement).
PbrDirectTerms eval_pbr_direct_bsdf(Surface s, float3 v, float3 l, float NoL) {
    float3 hvec = safe_normalize(v + l);
    float NoH = saturate(dot(s.n, hvec));
    float LoH = saturate(dot(l, hvec));
    float  D = D_GGX(NoH, s.a);
    float  V = V_SmithGGXCorrelated(s.NoV, NoL, s.a); // folds 1/(4 NoL NoV)
    float3 F = F_Schlick(LoH, s.f0);
    PbrDirectTerms r;
    r.specular = (D * V) * F * s.energy_comp; // PBR P0-D: multi-scatter energy comp
    r.diffuse = s.diffuse_color * (1.0 / PI);
    return r;
}

// eval_pbr_ambient_hemi — the sky/ground hemisphere ambient term (PBR P0-B / the PBR
// metal fix): Lambert diffuse against the up-axis hemisphere lerp, plus a
// reflection-vector-sampled specular tint (a metal mirrors its surroundings, not a
// flat sky tint), with DECOUPLED AO — diffuse ambient darkens with `ao_final`, but
// ambient specular darkens only with the roughness-aware `spec_ao` (a metal's
// diffuse is 0, so its ambient specular is its entire appearance and must not be
// AO-darkened like matte paint). TOKEN-FOR-TOKEN identical to the `LIGHT_KIND_SKY`
// block of `main()`'s light loop. Pure function of its parameters (no texture reads).
float3 eval_pbr_ambient_hemi(Surface s, float3 R, float2 dfg, float3 sky_color,
                              float3 ground_color, float hemi, float ao_final, float spec_ao) {
    float3 hemi_color = lerp(ground_color, sky_color, hemi);
    float  refl_hemi = dot(R, LIGHT_UP) * 0.5 + 0.5; // same up-axis as `hemi`
    // PBR metal fix: steepen the reflected hemisphere (smoothstep) so a metal sweeps a
    // real bright-cap -> dark-belly gradient instead of a flat mid-tone. The DIFFUSE
    // `hemi` stays LINEAR — only the specular lobe steepens.
    refl_hemi = refl_hemi * refl_hemi * (3.0 - 2.0 * refl_hemi);
    float3 refl_color = lerp(ground_color, sky_color, refl_hemi);
    float3 spec_ambient = (s.f0 * dfg.x + dfg.y) * refl_color * s.energy_comp;
    float3 diff_ambient = s.diffuse_color * hemi_color;
    return diff_ambient * ao_final + spec_ambient * spec_ao;
}

// eval_pbr_sun_disc — the PBR P1 HDR sun-disc term: a second, roughness-widened
// specular response from a directional light sampled along the REFLECTION vector `R`
// (not the direct Cook-Torrance half-vector lobe) — the chrome cue a flat sky
// gradient alone cannot produce. TOKEN-FOR-TOKEN identical to the
// `LIGHT_KIND_DIRECTIONAL` sun-disc block of `main()`'s light loop, EXCEPT the final
// `* SUN_ENV_WEIGHT` multiply stays at the call site: that constant (and
// `sun_kernel`, which produces `sun_k`) is declared AFTER this header's `#include`
// point in `deferred_pbr.hlsl`, so this header cannot reference it without a forward
// declaration. `sun_k` is the caller-evaluated `sun_kernel(R, l, alpha)`.
float3 eval_pbr_sun_disc(Surface s, float2 dfg, float sun_k, float3 light_color) {
    return (s.f0 * dfg.x + dfg.y) * light_color * sun_k * s.energy_comp;
}

// --- Rung-4b: the output-stage tonemap operators (VERBATIM cut of the resolve's "PBR P0-C"
// span) — shared by every render path's final `lit` write ------------------------------------

// === PBR P0-C — Stephen Hill ACES-fitted filmic tonemap + manual gamma-2.2 OETF ===============
//
// Pre-P0 the accumulated linear radiance was stored via a RAW `clamp(lit, 0, 1)`: a peaked
// GGX highlight clips to a flat white disk, destroying the Fresnel edge-tint that reads as
// metal. This fit (the SAME matrices/rational form Bevy `AcesFitted` / Godot use) rolls off
// highlights while staying near-identity in the midtones and preserving hue in the shoulder,
// so a bright specular highlight fades toward white gracefully instead of clipping.
//
// OETF verification (blocking, see the PBR P0 batch report): `gLit` (R8G8B8A8_UNORM) and the
// swapchain (`pick_surface_format` in `present/surface.rs` tries `*_UNORM` BEFORE `*_SRGB`, so
// a device advertising both — every consumer GPU — picks UNORM) are linear UNORM end to end;
// the present-blit (`fullscreen_sample.fs.hlsl`) is a raw `Sample` passthrough with no format
// reinterpretation. Nothing in this chain hardware-encodes sRGB, so a `lit` writer must
// gamma-encode itself here (one manual `pow(lit, 1/2.2)` after the tonemap) or the whole frame
// reads too dark on display. The host oracle mirrors both ops in the SAME order
// (`aces_fitted` then the gamma power) — see `tonemap_and_oetf` in `goldens.rs`.
static const float3x3 ACES_IN  = { 0.59719, 0.35458, 0.04823,  0.07600, 0.90834, 0.01566,  0.02840, 0.13383, 0.83777 };
static const float3x3 ACES_OUT = { 1.60475, -0.53108, -0.07367, -0.10208, 1.10813, -0.00605, -0.00327, -0.07276, 1.07602 };
static const float OETF_GAMMA_EXP = 1.0 / 2.2;

float3 aces_fitted(float3 c) {
    c = mul(ACES_IN, c);
    float3 a = c * (c + 0.0245786) - 0.000090537;
    float3 b = c * (0.983729 * c + 0.4329510) + 0.238081;
    return saturate(mul(ACES_OUT, a / b));
}

// Khronos PBR Neutral — LUT-free, hue-preserving, gentle toe. Linear Rec.709 in →
// linear[0,1] out (no gamma; the shared OETF follows). Source: KhronosGroup/ToneMapping.
float3 khronos_pbr_neutral(float3 color) {
    const float startCompression = 0.8 - 0.04; // 0.76
    const float desaturation     = 0.15;
    float x = min(color.r, min(color.g, color.b));
    float offset = (x < 0.08) ? (x - 6.25 * x * x) : 0.04;
    color -= offset;
    float peak = max(color.r, max(color.g, color.b));
    if (peak < startCompression) return color;
    const float d = 1.0 - startCompression; // 0.24
    float newPeak = 1.0 - d * d / (peak + d - startCompression);
    color *= newPeak / peak;
    float g = 1.0 - 1.0 / (desaturation * (peak - newPeak) + 1.0);
    return lerp(color, newPeak.xxx, g);
}

// Reinhard-Jodie — cheap hybrid, hue-preserving. Linear in → linear[0,1] out.
float3 reinhard_jodie(float3 v) {
    float l = dot(v, float3(0.2126, 0.7152, 0.0722)); // Rec.709 luma
    float3 tv = v / (1.0 + v);
    return saturate(lerp(v / (1.0 + l), tv, tv));
}

// Curve selector — the ACES arm calls the UNCHANGED aces_fitted, so mode 0 is
// bit-identical to today's expression. Unknown modes fall through to ACES.
float3 tonemap_select(float3 c, uint mode) {
    if (mode == TONEMAP_NEUTRAL)         return khronos_pbr_neutral(c);
    if (mode == TONEMAP_REINHARD_JODIE)  return reinhard_jodie(c);
    return aces_fitted(c); // TONEMAP_ACES (0) and any unknown → today's curve
}
