//! The screen-space AO (HBAO-lite, no-trig) horizon reducer leaf — Render P7 GROUP A.
//!
//! `sdf_ssao` (`shaders/sdf_ssao.comp.hlsl`) gathers a deterministic, full-resolution
//! horizon-based ambient-occlusion estimate from the FROZEN G-buffer: for each of
//! `SSAO_SLICES` rotated screen-space slices it marches `SSAO_STEPS` forward-projected
//! neighbour taps in each of the two `±dir` half-slices, tracking the maximum horizon
//! cosine, and folds the per-slice occlusions into a single `ao` factor. The algorithm
//! is the HBAO horizon-MAX reducer (NOT the GTAO arc integral): it needs only
//! `dot` / `max` / `sqrt` / `div`, so the host oracle is BIT-COMPARABLE (no `sin`/`cos`/
//! `acos` transcendental ULP gap, no `fract` integer-boundary discontinuity). See
//! `docs/RENDER-P7-SSAO-PLAN.md` ("Chosen algorithm").
//!
//! # ZERO new eDSL leaves
//!
//! `dot` is authored INLINE (`a.x()*b.x() + a.y()*b.y() + a.z()*b.z()` via the existing
//! [`Cf::vec3_x`]/[`Cf::vec3_y`]/[`Cf::vec3_z`] component reads + [`FieldScalar`]
//! `mul`/`add`), and `length = sqrt(dot(d, d))`. No `vec3_dot`/`vec3_cross`/
//! `vec3_normalize`/`floor`/`fract`/`sin`/`cos`/`acos`/`pow` leaf is added — so the
//! frozen marcher/field/shadow/brick/resolve `.spv` PHYSICALLY CANNOT fork (no enum /
//! printer change). The `ao*ao` strength power is an integer self-mul (`ao.mul(ao)`),
//! NOT `pow`.
//!
//! # The neighbour-fetch seam (the forward march)
//!
//! The per-tap forward neighbour-reconstruct `generate_ray(px', py') * gViewT'` (the
//! integer step-rounding + the bounds-clamp + the `mask != 1 || view_t >= 1e30` skip)
//! is HAND-WRITTEN glue (`sdf_ssao.comp.hlsl`), threaded into the generic body as the
//! `tap` closure — the SAME field-call-seam discipline [`crate::shadow`] uses for
//! `field`. On Emit, `tap` records a `float3` value node (the hand-written shader
//! supplies the reconstructed `P'`); on Eval (the CPU oracle), `tap` returns the
//! host-reconstructed neighbour position so [`ssao_estimate_body`]`::<EvalCf>` reproduces
//! the exact horizon reduction. A SKIPPED tap (out of bounds, or not an SDF/mesh-lit
//! pixel) is handled IN the seam: it returns a `P'` equal to the center `p` so the
//! tap's `falloff`/`sampleCos` contribute nothing — the eDSL carries no per-tap branch.
//!
//! The slice/step indices are threaded to the seam as the backend induction-variable
//! handle [`Cf::Iv`] (`usize` on Eval, the opaque SSA node on Emit): the host oracle
//! reads the real index, the emitter ignores it (the recorded body is unrolled once).
//!
//! Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — the CPU oracle (`f32` arithmetic + the host `tap` closure), the
//!     bit-comparable host mirror the golden gather calls.
//!   - `<EmitCf>` — the HLSL recorder ([`crate::emit::emit_hlsl_ssao`]) walking the STMT
//!     IR into the GENERATED span spliced into `sdf_ssao.comp.hlsl`.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The world-space SSAO sampling radius (`SSAO_RADIUS`). Beyond this the falloff zeroes a
/// tap's contribution. Spelled SYMBOLICALLY in the emitted HLSL (`SSAO_RADIUS`). The Eval
/// value is the owner default; it drives ONLY the host oracle (the `.spv` is unaffected by
/// the Eval value, which spells a symbol on Emit).
pub const SSAO_RADIUS: f32 = 0.5;

/// The number of rotated screen-space slices (`SSAO_SLICES`). Two slices × four steps × two
/// horizons = 16 taps, the no-blur single-frame floor. Spelled SYMBOLICALLY (`SSAO_SLICES`),
/// and used as the `[unroll]` slice-loop bound symbol.
pub const SSAO_SLICES: usize = 2;

/// The number of forward steps per half-slice (`SSAO_STEPS`). Spelled SYMBOLICALLY
/// (`SSAO_STEPS`) and used as the `[unroll]` step-loop bound symbol.
pub const SSAO_STEPS: usize = 4;

/// The occlusion strength multiplier (`SSAO_STRENGTH`) — scales the mean per-slice horizon
/// cosine before the `ao = 1 - strength*occ` complement. Spelled SYMBOLICALLY
/// (`SSAO_STRENGTH`).
pub const SSAO_STRENGTH: f32 = 1.0;

/// The `length(delta)` divide-by-zero guard (`SSAO_EPS`) — `sampleCos = dot(delta,dir) /
/// max(length(delta), SSAO_EPS)`. Spelled SYMBOLICALLY (`SSAO_EPS`).
pub const SSAO_EPS: f32 = 1.0e-4;

/// Accumulates ONE forward horizon tap into the running per-half-slice `horizonCos`
/// (`hc`). Authored over the control-flow axis `C`; the tapped neighbour world position
/// `pp` (`P'`) is supplied by the hand-written forward-reconstruct seam (see the module
/// doc), the center `p` (`P`) and the in-slice direction `dir` by the slice body.
///
/// The HBAO horizon step (the plan's `ssao_horizon_step`):
///   `delta     = P' - P`
///   `falloff   = clamp01(1 - dot(delta,delta) / (R*R))`   (the range gate)
///   `sampleCos = dot(delta, dir) / max(length(delta), SSAO_EPS)`
///   `hc        = max(hc, sampleCos * falloff)`
///
/// `dot` is INLINE (`delta.x*delta.x + ...`); `length = sqrt(dot(delta,delta))`. Returns
/// the updated `hc` value (a `Scalar`) — the slice body threads it through the steps.
#[inline]
pub fn ssao_horizon_step_body<C: Cf>(
    p: C::Vec3f,
    pp: C::Vec3f,
    dir: C::Vec3f,
    hc: C::Scalar,
) -> C::Scalar {
    // float3 delta = P' - P;  (a NAMED `float3 delta` temp — the committed materialization).
    let delta = C::temp_vec3("delta", C::vec3_sub(pp, p));

    // dot(delta, delta) = delta.x*delta.x + delta.y*delta.y + delta.z*delta.z  (INLINE dot).
    let dx = C::vec3_x(delta);
    let dy = C::vec3_y(delta);
    let dz = C::vec3_z(delta);
    let d2 = C::temp_float("d2", dx.mul(dx).add(dy.mul(dy)).add(dz.mul(dz)));

    // float r2 = SSAO_RADIUS * SSAO_RADIUS;  (a NAMED temp — materialized so the divisor
    // prints `d2 / r2`, NOT the precedence-wrong inline `d2 / SSAO_RADIUS * SSAO_RADIUS`
    // which HLSL parses left-to-right as `(d2 / SSAO_RADIUS) * SSAO_RADIUS`).
    let r = C::named_lit("SSAO_RADIUS", SSAO_RADIUS);
    let r2 = C::temp_float("r2", r.mul(r));
    // float falloff = clamp(1.0 - d2 / r2, 0.0, 1.0);
    let falloff = C::temp_float("falloff", C::Scalar::lit(1.0).sub(d2.div(r2)).clamp01());

    // dot(delta, dir) = delta.x*dir.x + delta.y*dir.y + delta.z*dir.z  (INLINE dot).
    let ddir = dx
        .mul(C::vec3_x(dir))
        .add(dy.mul(C::vec3_y(dir)))
        .add(dz.mul(C::vec3_z(dir)));

    // float sampleCos = dot(delta, dir) / max(sqrt(d2), SSAO_EPS);  (length(delta) =
    // sqrt(d2) — reuse the `d2` temp so the host/GPU sqrt operand is bit-identical, NOT a
    // recomputed component sum).
    let eps = C::named_lit("SSAO_EPS", SSAO_EPS);
    let sample_cos = C::temp_float("sampleCos", ddir.div(d2.sqrt().max(eps)));

    // hc = max(hc, sampleCos * falloff);  (the running horizon max; the slice body owns the
    // `hc` mutable local, so this returns the updated value to assign.)
    hc.max(sample_cos.mul(falloff))
}

/// Reduces ONE rotated slice's two `±dir` half-slices into an occlusion contribution
/// (`occ_slice`). Authored over `C`; the `STEPS` forward taps are supplied by the
/// hand-written `tap` seam, indexed by `(step_iv, sign)` so the eDSL stays branch-free
/// (the per-tap bounds/sentinel skip lives in the seam, see the module doc).
///
/// The plan's `ssao_slice`: a `+dir` half-slice horizon max over `STEPS` taps, then a
/// `-dir` half-slice horizon max, summed (`occ_slice = hc_pos + hc_neg`). `tap(step, false)`
/// is the `+dir` neighbour `P'`, `tap(step, true)` the `-dir` neighbour; both already carry
/// the forward-projected world position the seam reconstructed.
///
/// Returns the slice occlusion (a `Scalar`). The step loops are UNROLLED on the host
/// oracle and recorded as `[unroll] for` spans on Emit (via [`Cf::runtime_for`] with the
/// `SSAO_STEPS` bound symbol), so the emitted HLSL spells the two `for` loops the committed
/// shader carries.
#[inline]
pub fn ssao_slice_body<C: Cf, T: Fn(C::Iv, bool) -> C::Vec3f>(
    p: C::Vec3f,
    dir: C::Vec3f,
    tap: &T,
) -> C::Scalar {
    // float hc_pos = 0.0;  — the +dir half-slice horizon max accumulator.
    let hc_pos = C::decl_var("hc_pos", C::Scalar::lit(0.0));
    // The loop's `Flow` is discarded: the body never `ret`s (no early return), so on Eval it
    // always completes naturally (`Continue`) and on Emit `runtime_for` always returns
    // `Continue` after recording. (Shadow's IIFE consumes it via `?`; this body has none.)
    let _ = C::runtime_for("[unroll]", "sp", "SSAO_STEPS", SSAO_STEPS, |iv| -> Flow {
        // `iv` is the host `usize` step on Eval and the recorded SSA iv on Emit; the `tap`
        // seam reads it (Eval) or ignores it (Emit — it returns a per-call `Vec3Param`). The
        // unroll records the body ONCE (DXC unrolls it).
        let pp = tap(iv, false);
        C::set_var(
            &hc_pos,
            ssao_horizon_step_body::<C>(p, pp, dir, C::get_var(&hc_pos)),
        );
        Flow::Continue(())
    });

    // float hc_neg = 0.0;  — the -dir half-slice horizon max accumulator (the `-dir` taps).
    let hc_neg = C::decl_var("hc_neg", C::Scalar::lit(0.0));
    let neg_dir = C::vec3_mul_scalar(dir, C::Scalar::lit(-1.0));
    let _ = C::runtime_for("[unroll]", "sn", "SSAO_STEPS", SSAO_STEPS, |iv| -> Flow {
        let pp = tap(iv, true);
        C::set_var(
            &hc_neg,
            ssao_horizon_step_body::<C>(p, pp, neg_dir, C::get_var(&hc_neg)),
        );
        Flow::Continue(())
    });

    // occ_slice = hc_pos + hc_neg;  (the two half-slice horizon maxes summed).
    C::get_var(&hc_pos).add(C::get_var(&hc_neg))
}

/// Folds the `SSAO_SLICES` rotated slices into the final `ao` factor — the top-level
/// SSAO body the [`crate::emit::emit_hlsl_ssao`] entry traces. Authored over `C`; the
/// hand-written shader supplies the center world position `P` (`p`), the per-slice rotated
/// in-screen direction (`slice_dir`, a [`Cf::Iv`]-indexed seam), and the forward-tap seam
/// (`tap`, indexed by `(slice_iv, step_iv, sign)`).
///
/// The plan's `ssao_estimate`:
///   `occ = Σ_slices ssao_slice(...)`
///   `ao  = clamp01(1 - SSAO_STRENGTH * occ / SSAO_SLICES)`
///   `ao  = ao * ao`   (the integer self-mul strength power, NOT `pow`)
///
/// `slice_dir(s)` returns slice `s`'s rotated 3D in-screen direction (the hand-written
/// rotation-table lookup reconstructs it into a `float3`); `tap(s, step, sign)` the
/// forward-reconstructed neighbour `P'` for that slice's step. Returns the `ao` factor a
/// caller stores (the shader writes `ssao[px,py] = ao`).
#[inline]
pub fn ssao_estimate_body<C, D, T>(p: C::Vec3f, slice_dir: &D, tap: &T) -> C::Scalar
where
    C: Cf,
    D: Fn(C::Iv) -> C::Vec3f,
    T: Fn(C::Iv, C::Iv, bool) -> C::Vec3f,
{
    // float occ = 0.0;  — the slice-occlusion accumulator.
    let occ = C::decl_var("occ", C::Scalar::lit(0.0));
    // The slice loop's `Flow` is discarded for the same reason as the step loops (no `ret`).
    let _ = C::runtime_for("[unroll]", "sl", "SSAO_SLICES", SSAO_SLICES, |s| -> Flow {
        let dir = slice_dir(s);
        // Bind the slice index into the per-step seam so `ssao_slice_body`'s `tap(step,
        // sign)` resolves the right slice's neighbour.
        let slice_tap = |step: C::Iv, sign: bool| tap(s, step, sign);
        C::set_var(
            &occ,
            C::get_var(&occ).add(ssao_slice_body::<C, _>(p, dir, &slice_tap)),
        );
        Flow::Continue(())
    });

    // float ao = clamp(1.0 - SSAO_STRENGTH * occ / SSAO_SLICES_F, 0.0, 1.0);
    let strength = C::named_lit("SSAO_STRENGTH", SSAO_STRENGTH);
    let slices = C::named_lit("SSAO_SLICES_F", SSAO_SLICES as f32);
    let ao = C::temp_float(
        "ao",
        C::Scalar::lit(1.0)
            .sub(strength.mul(C::get_var(&occ)).div(slices))
            .clamp01(),
    );

    // ao = ao * ao;  (the integer self-mul strength power, NOT `pow(ao, 2)`).
    ao.mul(ao)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cf::EvalCf;

    /// Builds the `slice_dir` seam for a single fixed in-screen axis (both slices share the
    /// same axis here — the test exercises the horizon REDUCTION, not the rotation, which is
    /// the hand-written integer-hash glue out of scope for this leaf). `dir = (1, 0, 0)`.
    fn fixed_dir(_s: usize) -> [f32; 3] {
        [1.0, 0.0, 0.0]
    }

    /// Folds the estimate over a tiny synthetic neighbourhood. `tap(s, step, sign)` returns
    /// the neighbour world position `P'` for that tap; the center is the origin. `pos` builds
    /// the neighbour from the half-slice sign + step (the closure the host golden supplies).
    fn estimate<T: Fn(usize, usize, bool) -> [f32; 3]>(tap: T) -> f32 {
        let p = [0.0f32, 0.0, 0.0];
        ssao_estimate_body::<EvalCf, _, _>(p, &fixed_dir, &tap)
    }

    #[test]
    fn deep_crevice_neighbourhood_darkens_ao() {
        // A concave corner: every forward tap RISES toward the camera along the slice axis
        // (`+dir`) — `delta` has a strong positive projection on `dir`, well within the
        // SSAO_RADIUS falloff, so each tap's `sampleCos * falloff` is large -> the horizon max
        // is high -> `occ` is large -> `ao` clearly < 1.
        let tap = |_s: usize, step: usize, sign: bool| -> [f32; 3] {
            // March outward in pixel steps; the wall climbs in +x (the dir axis). The -dir
            // half-slice sees the same rising wall mirrored, so both horizons are occluded.
            let d = 0.06 * (step as f32 + 1.0); // < SSAO_RADIUS (0.5): falloff > 0
            let x = if sign { -d } else { d };
            // The neighbour is displaced along the slice axis (occluding) — the crevice wall.
            [x, 0.0, 0.0]
        };
        let ao = estimate(tap);
        assert!(
            ao < 0.85,
            "a deep-crevice neighbourhood must darken AO well below 1.0, got ao = {ao}"
        );
        assert!(ao >= 0.0, "AO is clamped to [0,1], got ao = {ao}");
    }

    #[test]
    fn flat_open_neighbourhood_keeps_ao_near_one() {
        // A flat / open region: every neighbour lies in the surface plane PERPENDICULAR to the
        // slice axis (`delta` along +z/-z, with `dir = +x`), so `dot(delta, dir) == 0` ->
        // `sampleCos == 0` -> no horizon is raised -> `occ == 0` -> `ao == 1`.
        let tap = |_s: usize, step: usize, sign: bool| -> [f32; 3] {
            let d = 0.06 * (step as f32 + 1.0);
            let z = if sign { -d } else { d };
            [0.0, 0.0, z] // perpendicular to dir = (1,0,0): zero horizon cosine
        };
        let ao = estimate(tap);
        assert!(
            ao > 0.999,
            "a flat/open neighbourhood must leave AO ~= 1.0, got ao = {ao}"
        );
    }

    #[test]
    fn no_neighbours_is_fully_unoccluded() {
        // Degenerate: every tap reconstructs `Pp == P` (the seam's out-of-bounds / non-lit
        // skip). `delta == 0` -> `sampleCos == 0` (guarded by SSAO_EPS, no NaN) -> `ao == 1`.
        let ao = estimate(|_s, _step, _sign| [0.0, 0.0, 0.0]);
        assert!(
            (ao - 1.0).abs() < 1.0e-6,
            "an all-skipped neighbourhood must be fully unoccluded (ao = 1.0), got ao = {ao}"
        );
    }
}
