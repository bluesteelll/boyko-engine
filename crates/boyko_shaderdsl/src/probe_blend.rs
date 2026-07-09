//! The SDFDDGI I2 octahedral-tile BLEND leaves — the per-ray cosine irradiance accumulate
//! and the two-moment depth accumulate.
//!
//! The I2 probe-update pass gathers each probe's cached rays into its octahedral tiles PER
//! TEXEL: for one oct texel (decoded direction `texelDir`) it weights every cached ray by the
//! clamped cosine `w = max(0, dot(texelDir, rayDir))` and accumulates the ray's radiance /
//! marched distance. These two leaves author ONE ray's contribution:
//!
//! - [`probe_blend_body`] — the IRRADIANCE cosine accumulate (`sum_rgb += L * w`, `sum_w +=
//!   w`). The tile write (after all rays) is `irr = sum_rgb / max(sum_w, DDGI_MIN_SUM_WEIGHT)`,
//!   reusing the resolve-side `1e-6` guard (I0b `goldens::DDGI_MIN_SUM_WEIGHT`) — that divide
//!   is hand-written glue, not this span.
//! - [`probe_depth_blend_body`] — the TWO-MOMENT depth accumulate (`dmean += w * t`, `dmean2
//!   += w * t * t`) over the SAME cosine `w` and the marched hit distance `t`. No `pow` this
//!   rung (the depth-sharpen `pow` is an I4 Chebyshev knob — plan §1.5).
//!
//! # ZERO new eDSL leaves (the SSAO discipline)
//!
//! `dot(texelDir, rayDir)` is authored INLINE (`a.x*b.x + a.y*b.y + a.z*b.z` via the existing
//! [`Cf::vec3_x`]/[`Cf::vec3_y`]/[`Cf::vec3_z`] component reads + [`FieldScalar`] `mul`/`add`) —
//! NO `vec3_dot` leaf, no frozen printer fork (the exact discipline [`crate::ssao`] uses). So
//! the frozen marcher/field/shadow/brick/resolve `.spv` PHYSICALLY CANNOT fork.
//!
//! # The `float3` radiance accumulator
//!
//! `sum_rgb` is a `float3` running sum. The eDSL carries it component-wise: each leaf takes the
//! three scalar accumulator lanes plus `sum_w` and returns the four updated lanes, so the
//! hand-written per-texel glue owns the `float3 sum_rgb` / `float sum_w` mutable locals and the
//! spliced span reads/writes them by name (the SSAO `hc`-accumulator threading shape).
//!
//! Update is GPU-golden + tolerance (the marched radiance is not bit-exact — the resolve
//! sample is the bit-exact path, D6), so no host oracle bit-pin is required this rung; the
//! `<EvalCf>` instantiation is only the unit-test fixture (all-equal rays → uniform; a single
//! ray → a cosine peak at the aligned texel).

use crate::cf::Cf;
use crate::scalar::FieldScalar;

/// The clamped-cosine weight of ONE ray against one oct-texel direction (`w = max(0,
/// dot(texelDir, rayDir))`) — the shared weight both blend leaves fold. `texel_dir` is the
/// decoded texel direction (from `oct_decode`); `ray_dir` the cached ray direction. `dot` is
/// the INLINE component fold (the SSAO discipline — no `vec3_dot` leaf).
#[inline]
fn cosine_weight<C: Cf>(texel_dir: C::Vec3f, ray_dir: C::Vec3f) -> C::Scalar {
    // dot(texelDir, rayDir) = texelDir.x*rayDir.x + texelDir.y*rayDir.y + texelDir.z*rayDir.z.
    let d = C::vec3_x(texel_dir)
        .mul(C::vec3_x(ray_dir))
        .add(C::vec3_y(texel_dir).mul(C::vec3_y(ray_dir)))
        .add(C::vec3_z(texel_dir).mul(C::vec3_z(ray_dir)));
    // w = max(0.0, dot) — a back-facing ray (dot < 0) contributes nothing to this texel.
    d.max(C::Scalar::lit(0.0))
}

/// Accumulates ONE ray's cosine-weighted radiance into a probe irradiance texel's running sums
/// (plan §1.3). Authored ONCE over the control-flow axis `C`. `texel_dir` is the oct-decoded
/// texel direction, `ray_dir` the cached ray direction, `radiance` the ray's shaded radiance
/// `L` (its three lanes `l_r`/`l_g`/`l_b`), and `(sum_r, sum_g, sum_b, sum_w)` the running
/// accumulator lanes. Returns the FOUR updated lanes `(sum_r', sum_g', sum_b', sum_w')` — the
/// hand-written per-texel glue owns the mutable `float3 sum_rgb` / `float sum_w` locals and
/// assigns the returned lanes back.
///
/// The math is `w = max(0, dot(texelDir, rayDir))`, then `sum_rgb += L * w`, `sum_w += w`. The
/// weight `w` is materialized once (a NAMED `float w` temp) so both the RGB scale and the `sum_w`
/// add read the SAME value, matching the committed shader's single `dot`.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn probe_blend_body<C: Cf>(
    texel_dir: C::Vec3f,
    ray_dir: C::Vec3f,
    l_r: C::Scalar,
    l_g: C::Scalar,
    l_b: C::Scalar,
    sum_r: C::Scalar,
    sum_g: C::Scalar,
    sum_b: C::Scalar,
    sum_w: C::Scalar,
) -> (C::Scalar, C::Scalar, C::Scalar, C::Scalar) {
    // float w = max(0.0, dot(texelDir, rayDir));  (a NAMED temp — the single cosine both the
    // RGB scale and the weight-sum read).
    let w = C::temp_float("w", cosine_weight::<C>(texel_dir, ray_dir));

    // sum_rgb += L * w;  — component-wise `sum_c = sum_c + L_c * w`.
    let sum_r2 = sum_r.add(l_r.mul(w));
    let sum_g2 = sum_g.add(l_g.mul(w));
    let sum_b2 = sum_b.add(l_b.mul(w));
    // sum_w += w;
    let sum_w2 = sum_w.add(w);

    (sum_r2, sum_g2, sum_b2, sum_w2)
}

/// Accumulates ONE ray's cosine-weighted DEPTH moments into a probe depth texel's running sums
/// (plan §1.5) — the two-moment (mean, mean-of-squares) visibility tile the I4 Chebyshev
/// leak-suppression reads. Authored ONCE over `C`. `texel_dir` is the oct-decoded texel
/// direction, `ray_dir` the cached ray direction, `t` the ray's marched hit distance (or
/// `GI_T_MAX` on a sky miss), and `(dmean, dmean2, dw)` the running accumulator lanes. Returns
/// the THREE updated lanes `(dmean', dmean2', dw')`.
///
/// The math is `w = max(0, dot(texelDir, rayDir))`, then `dmean += w * t`, `dmean2 += w * t *
/// t`, `dw += w`. NO `pow` (the depth-sharpen `pow` is an I4 knob — plan §1.5). The written
/// tile (hand-written glue) is `(dmean / dw, dmean2 / dw)`. `w` and `w * t` are materialized as
/// NAMED temps so `dmean` and `dmean2` read the same weighted distance (`dmean2` folds the
/// extra `* t`), matching the committed shader.
#[inline]
pub fn probe_depth_blend_body<C: Cf>(
    texel_dir: C::Vec3f,
    ray_dir: C::Vec3f,
    t: C::Scalar,
    dmean: C::Scalar,
    dmean2: C::Scalar,
    dw: C::Scalar,
) -> (C::Scalar, C::Scalar, C::Scalar) {
    // float w = max(0.0, dot(texelDir, rayDir));
    let w = C::temp_float("w", cosine_weight::<C>(texel_dir, ray_dir));
    // float wt = w * t;  (the weighted distance — reused by both moments so `dmean2` folds the
    // extra `* t` off the SAME product).
    let wt = C::temp_float("wt", w.mul(t));

    // dmean  += w * t;
    let dmean_out = dmean.add(wt);
    // dmean2 += w * t * t;   (= wt * t — the second moment).
    let dmean2_out = dmean2.add(wt.mul(t));
    // dw += w;
    let dw_out = dw.add(w);

    (dmean_out, dmean2_out, dw_out)
}
