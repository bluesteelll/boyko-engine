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
// across every render path. Rung-0 extracts only the primitives that are PURE
// functions of their parameters (no G-buffer/cbuffer reads); the `eval_pbr_direct`/
// `eval_pbr_ambient` public surface (§C) lands in a later rung once the other paths
// need it.
//
// # INCLUDE CONTRACT (precondition)
//
// `static const float PI` MUST be declared in the including TU BEFORE this header is
// `#include`d — `D_GGX` below reads it. The including TU owns the constant; this
// header references it (the same pattern `sdf_field.hlsli` uses for its `Buf`
// precondition).

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
