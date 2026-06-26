//! The screen-space AO (HBAO-lite, no-trig) horizon reducer leaf — Render P7 GROUP A.
//!
//! `sdf_ssao` (`shaders/sdf_ssao.comp.hlsl`) gathers a deterministic, full-resolution
//! horizon-based ambient-occlusion estimate from the FROZEN G-buffer: for each of
//! `SSAO_SLICES` rotated screen-space slices it marches `SSAO_STEPS` forward-projected
//! neighbour taps in each of the two half-slices, tracking the maximum neighbour ELEVATION
//! ABOVE THE SURFACE TANGENT PLANE (measured against the center surface normal `N`), and
//! folds the per-slice occlusions into a single `ao` factor. The algorithm is the HBAO
//! horizon-MAX reducer (NOT the GTAO arc integral): it needs only `dot` / `max` / `sqrt` /
//! `div`, so the host oracle is BIT-COMPARABLE (no `sin`/`cos`/`acos` transcendental ULP
//! gap, no `fract` integer-boundary discontinuity). See `docs/RENDER-P7-SSAO-PLAN.md`
//! ("Chosen algorithm").
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
//! tap's `falloff`/`elev` contribute nothing — the eDSL carries no per-tap branch.
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
pub const SSAO_STRENGTH: f32 = 2.5;

/// The `length(delta)` divide-by-zero guard (`SSAO_EPS`) — `elev = max(dot(delta,N) /
/// max(length(delta), SSAO_EPS), 0)`. Spelled SYMBOLICALLY (`SSAO_EPS`).
pub const SSAO_EPS: f32 = 1.0e-4;

/// One SSAO QUALITY PRESET — the per-variant tuning the Render P7-Q2 PRE-COMPILED `.spv`
/// variants (Mechanism C) BAKE as `static const` so every `[unroll]` slice/step loop stays
/// fully unrolled with ZERO per-pixel runtime cost. A variant is selected at runtime by
/// binding a different pipeline, NEVER by a dynamic loop bound (Mechanism A — the de-unroll
/// tax — is rejected).
///
/// `slices` × `steps` × 2 horizons is the per-pixel tap budget; `radius`/`strength`/`eps`
/// are the scalars the generated horizon-step span spells SYMBOLICALLY (so the span text is
/// IDENTICAL across all variants — only this header block + the loop bounds change). The
/// resolve blur radius is NOT part of a preset (it stays one fixed value in
/// `deferred_pbr.hlsl`, the resolve is not variantized).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsaoParams {
    /// The world-space sampling radius (`SSAO_RADIUS`).
    pub radius: f32,
    /// The number of rotated screen-space slices (`SSAO_SLICES`).
    pub slices: u32,
    /// The number of forward steps per half-slice (`SSAO_STEPS`).
    pub steps: u32,
    /// The occlusion strength multiplier (`SSAO_STRENGTH`).
    pub strength: f32,
    /// The `length(delta)` divide-by-zero guard (`SSAO_EPS`).
    pub eps: f32,
}

/// The SSAO quality-preset table (Render P7-Q2 — Mechanism C). One PRE-COMPILED `.spv`
/// variant per row; the host selects a variant by binding its pipeline. The Medium row
/// (`SSAO_PRESETS[1]`) MUST equal today's shipped scalars (`SSAO_RADIUS`/`SSAO_SLICES`/…)
/// — it is the no-op proof: re-emitting + re-DXCing the Medium variant reproduces the
/// committed `sdf_ssao.comp.spv` byte-for-byte.
///
/// Only the GLUE `static const` header (and thereby the `[unroll]` loop bounds) varies per
/// row; the eDSL-GENERATED horizon-step span text is byte-identical across all three (it
/// spells `SSAO_RADIUS`/`SSAO_EPS` symbolically — see [`ssao_horizon_step_body`]).
pub const SSAO_PRESETS: [SsaoParams; 3] = [
    // Low — the cheapest tap budget (2 slices × 3 steps × 2 = 12 taps).
    SsaoParams {
        radius: 0.5,
        slices: 2,
        steps: 3,
        strength: 2.5,
        eps: 1.0e-4,
    },
    // Medium — IDENTICAL to today's shipped consts (2 slices × 4 steps × 2 = 16 taps). The
    // no-op proof: this variant's `.spv` == the committed `sdf_ssao.comp.spv`.
    SsaoParams {
        radius: SSAO_RADIUS,
        slices: SSAO_SLICES as u32,
        steps: SSAO_STEPS as u32,
        strength: SSAO_STRENGTH,
        eps: SSAO_EPS,
    },
    // High — the widest tap budget (3 slices × 6 steps × 2 = 36 taps).
    SsaoParams {
        radius: 0.5,
        slices: 3,
        steps: 6,
        strength: 2.5,
        eps: 1.0e-4,
    },
];

/// The variant-quality index in [`SSAO_PRESETS`] (the table-row name). Also the canonical
/// per-variant file-stem suffix (`sdf_ssao_<quality>.comp.{hlsl,spv}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SsaoQuality {
    /// `SSAO_PRESETS[0]` — `sdf_ssao_low`.
    Low,
    /// `SSAO_PRESETS[1]` — `sdf_ssao_medium` (== today's shipped consts).
    Medium,
    /// `SSAO_PRESETS[2]` — `sdf_ssao_high`.
    High,
}

impl SsaoQuality {
    /// All three qualities in table order — the iteration source for the per-variant
    /// emit + byte-identity loop.
    pub const ALL: [SsaoQuality; 3] = [SsaoQuality::Low, SsaoQuality::Medium, SsaoQuality::High];

    /// The lowercase file-stem suffix (`"low"`/`"medium"`/`"high"`) — the variant
    /// `sdf_ssao_<suffix>.comp.{hlsl,spv}` stem.
    #[inline]
    pub const fn suffix(self) -> &'static str {
        match self {
            SsaoQuality::Low => "low",
            SsaoQuality::Medium => "medium",
            SsaoQuality::High => "high",
        }
    }

    /// The preset row this quality selects from [`SSAO_PRESETS`].
    #[inline]
    pub const fn params(self) -> SsaoParams {
        SSAO_PRESETS[self as usize]
    }
}

/// Renders the per-variant SSAO `static const` tuning header — the GLUE block that BAKES one
/// [`SsaoParams`] preset into the `.hlsl` (Render P7-Q2 Mechanism C). This is the ONLY text
/// that varies between the pre-compiled variants: the eDSL-generated horizon-step span spells
/// `SSAO_RADIUS`/`SSAO_EPS` symbolically and is byte-identical across all variants, and the
/// `[unroll]` slice/step loops read `SSAO_SLICES`/`SSAO_STEPS` from this header.
///
/// The block is byte-identical to the committed `sdf_ssao.comp.hlsl:110-115` when `p ==
/// SSAO_PRESETS[1]` (Medium) — that exact reproduction is the no-op proof the byte-identity
/// loop asserts. The float spellings match the committed literals: `radius`/`strength` print
/// via the default `f32` `Display` (`0.5` / `2.5`), `eps` prints as `1.0e-4`, and the slice
/// count emits both the `uint` (`Nu`) and the `SSAO_SLICES_F` float (`N.0`) the `occ / N`
/// divisor reads. Counts are integers, so they spell exactly (`2u`/`3u`/`4u`/`6u`).
pub fn ssao_glue_header(p: &SsaoParams) -> String {
    // The `eps` is spelled `1.0e-4` (the committed literal), NOT the `f32` `Display`
    // `0.0001`: the presets all share `1.0e-4`, so a fixed spelling reproduces the committed
    // header byte-for-byte. If a future preset needs a different `eps`, extend this.
    debug_assert!(
        p.eps == 1.0e-4,
        "invariant: SSAO presets currently all use eps = 1.0e-4 (the committed `1.0e-4` literal)"
    );
    let radius = fmt_hlsl_f32(p.radius);
    let strength = fmt_hlsl_f32(p.strength);
    let slices_f = fmt_hlsl_f32(p.slices as f32);
    format!(
        "static const float SSAO_RADIUS   = {radius};     // world-space sampling radius\n\
         static const uint  SSAO_SLICES   = {slices}u;      // rotated screen-space slices\n\
         static const float SSAO_SLICES_F = {slices_f};     // the slice count as a float (the `occ / N` divisor)\n\
         static const uint  SSAO_STEPS    = {steps}u;      // forward steps per half-slice\n\
         static const float SSAO_STRENGTH = {strength};     // occlusion strength multiplier\n\
         static const float SSAO_EPS      = 1.0e-4;  // length(delta) divide-by-zero guard\n",
        slices = p.slices,
        steps = p.steps,
    )
}

/// Spells an `f32` the way the committed SSAO header literals read: an integral-valued
/// scalar gets a trailing `.0` (`2.0`), otherwise the default `f32` `Display` (`0.5`/`2.5`).
/// This matches the committed `SSAO_RADIUS = 0.5` / `SSAO_SLICES_F = 2.0` / `SSAO_STRENGTH =
/// 2.5` spellings so the Medium header reproduces byte-for-byte.
fn fmt_hlsl_f32(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

/// The first line of the committed SSAO `static const` tuning block — the anchor
/// [`variant_hlsl`] swaps FROM (inclusive). The base shader carries exactly one such line.
const SSAO_HEADER_FIRST_LINE: &str = "static const float SSAO_RADIUS";

/// The last line of the committed SSAO `static const` tuning block — the anchor
/// [`variant_hlsl`] swaps TO (inclusive). The base shader carries exactly one such line.
const SSAO_HEADER_LAST_LINE: &str = "static const float SSAO_EPS";

/// Produces ONE quality variant's complete `.hlsl` from the committed base SSAO shader text
/// by swapping ONLY the `static const SSAO_*` tuning header (the 6 lines from `SSAO_RADIUS`
/// through `SSAO_EPS`) for the per-preset header ([`ssao_glue_header`]). Everything else —
/// the eDSL-GENERATED horizon-step span, the forward neighbour reconstruct, the rotation/
/// step-phase dither, the `[unroll]` slice/step loops (which read the swapped `SSAO_SLICES`/
/// `SSAO_STEPS`), and the `occ → ao` fold — is carried VERBATIM from `base`.
///
/// This is the single-source seam: the glue body lives ONCE in the base file, so the Medium
/// variant (`p == SSAO_PRESETS[1]`) is byte-identical to the base (the no-op proof), and the
/// generated span text is identical across all variants. The `base` line ending is detected
/// and preserved (CRLF or LF), so the swap does not perturb bytes around the block.
///
/// Panics if the header anchors are missing or out of order (a malformed base shader — a
/// developer-tool invariant, surfaced loudly rather than silently producing a broken variant).
pub fn variant_hlsl(base: &str, params: SsaoParams) -> String {
    // Detect the base's line ending so the swapped header matches it (the committed base is
    // LF; a CRLF checkout must round-trip CRLF). `ssao_glue_header` emits LF, normalized below.
    let crlf = base.contains("\r\n");

    let first = base.find(SSAO_HEADER_FIRST_LINE).unwrap_or_else(|| {
        panic!(
            "invariant: base SSAO shader is missing the `{SSAO_HEADER_FIRST_LINE}` header anchor"
        )
    });
    // The block END is the line-end of the LAST anchor line: find the anchor, then the next
    // line break after it (inclusive of that break, so the replacement owns its trailing EOL).
    let last_start = base[first..]
        .find(SSAO_HEADER_LAST_LINE)
        .map(|off| first + off)
        .unwrap_or_else(|| {
            panic!(
                "invariant: base SSAO shader is missing the `{SSAO_HEADER_LAST_LINE}` header \
                 anchor after `{SSAO_HEADER_FIRST_LINE}`"
            )
        });
    let after_last = base[last_start..]
        .find('\n')
        .map(|off| last_start + off + 1) // include the '\n'
        .expect(
            "invariant: the SSAO_EPS header line must be newline-terminated in the base shader",
        );

    let mut header = ssao_glue_header(&params);
    if crlf {
        header = header.replace('\n', "\r\n");
    }

    let mut out = String::with_capacity(base.len() + header.len());
    out.push_str(&base[..first]);
    out.push_str(&header);
    out.push_str(&base[after_last..]);
    out
}

/// Accumulates ONE forward horizon tap into the running per-half-slice `horizonCos`
/// (`hc`). Authored over the control-flow axis `C`; the tapped neighbour world position
/// `pp` (`P'`) is supplied by the hand-written forward-reconstruct seam (see the module
/// doc), the center `p` (`P`) and the center surface normal `n` (`N`) by the slice body.
///
/// The HBAO horizon step (the plan's `ssao_horizon_step`) measures the neighbour's
/// ELEVATION ABOVE THE SURFACE TANGENT PLANE (the sine of the elevation angle against the
/// center normal `N`), clamped to non-negative so only neighbours ABOVE the tangent occlude:
///   `delta     = P' - P`
///   `falloff   = clamp01(1 - dot(delta,delta) / (R*R))`   (the range gate)
///   `elev      = max(dot(delta, N) / max(length(delta), SSAO_EPS), 0.0)`
///   `hc        = max(hc, elev * falloff)`
///
/// A flat surface (`delta ⊥ N` → `elev = 0`) raises NO horizon (`hc = 0` → AO = 1.0); a
/// crevice (neighbours rising above the tangent toward the camera → `dot(delta, N) > 0`)
/// raises `elev > 0` → AO < 1. `dot` is INLINE (`delta.x*N.x + ...`); `length =
/// sqrt(dot(delta,delta))`. Returns the updated `hc` value (a `Scalar`) — the slice body
/// threads it through the steps.
#[inline]
pub fn ssao_horizon_step_body<C: Cf>(
    p: C::Vec3f,
    pp: C::Vec3f,
    n: C::Vec3f,
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

    // dot(delta, N) = delta.x*N.x + delta.y*N.y + delta.z*N.z  (INLINE dot) — the
    // unnormalized elevation against the center surface normal.
    let dn = dx
        .mul(C::vec3_x(n))
        .add(dy.mul(C::vec3_y(n)))
        .add(dz.mul(C::vec3_z(n)));

    // float elev = max(dot(delta, N) / max(sqrt(d2), SSAO_EPS), 0.0);  the sine of the
    // elevation above the tangent plane, clamped non-negative so neighbours BELOW the
    // tangent do not occlude. `length(delta) = sqrt(d2)` reuses the `d2` temp so the host/GPU
    // sqrt operand is bit-identical, NOT a recomputed component sum.
    let eps = C::named_lit("SSAO_EPS", SSAO_EPS);
    let elev = C::temp_float(
        "elev",
        dn.div(d2.sqrt().max(eps)).max(C::Scalar::lit(0.0)),
    );

    // hc = max(hc, elev * falloff);  (the running horizon max; the slice body owns the
    // `hc` mutable local, so this returns the updated value to assign.)
    hc.max(elev.mul(falloff))
}

/// Reduces ONE rotated slice's two half-slices into an occlusion contribution
/// (`occ_slice`). Authored over `C`; the `STEPS` forward taps are supplied by the
/// hand-written `tap` seam, indexed by `(step_iv, sign)` so the eDSL stays branch-free
/// (the per-tap bounds/sentinel skip lives in the seam, see the module doc).
///
/// The plan's `ssao_slice`: a `+` half-slice horizon max over `STEPS` taps, then a `-`
/// half-slice horizon max, summed (`occ_slice = hc_pos + hc_neg`). `tap(step, false)` is
/// the `+` neighbour `P'`, `tap(step, true)` the `-` neighbour; both already carry the
/// forward-projected world position the seam reconstructed. The screen slice DIRECTION is
/// folded into the `tap` seam (it picks the neighbour pixel); the horizon math measures
/// elevation against the CENTER SURFACE NORMAL `n`, the SAME for both half-slices.
///
/// Returns the slice occlusion (a `Scalar`). The step loops are UNROLLED on the host
/// oracle and recorded as `[unroll] for` spans on Emit (via [`Cf::runtime_for`] with the
/// `SSAO_STEPS` bound symbol), so the emitted HLSL spells the two `for` loops the committed
/// shader carries.
#[inline]
pub fn ssao_slice_body<C: Cf, T: Fn(C::Iv, bool) -> C::Vec3f>(
    p: C::Vec3f,
    n: C::Vec3f,
    tap: &T,
) -> C::Scalar {
    // float hc_pos = 0.0;  — the + half-slice horizon max accumulator.
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
            ssao_horizon_step_body::<C>(p, pp, n, C::get_var(&hc_pos)),
        );
        Flow::Continue(())
    });

    // float hc_neg = 0.0;  — the - half-slice horizon max accumulator (the `-` taps). The
    // center normal `n` is the SAME for both half-slices (the elevation reference is the
    // surface tangent plane, not the screen slice direction).
    let hc_neg = C::decl_var("hc_neg", C::Scalar::lit(0.0));
    let _ = C::runtime_for("[unroll]", "sn", "SSAO_STEPS", SSAO_STEPS, |iv| -> Flow {
        let pp = tap(iv, true);
        C::set_var(
            &hc_neg,
            ssao_horizon_step_body::<C>(p, pp, n, C::get_var(&hc_neg)),
        );
        Flow::Continue(())
    });

    // occ_slice = hc_pos + hc_neg;  (the two half-slice horizon maxes summed).
    C::get_var(&hc_pos).add(C::get_var(&hc_neg))
}

/// Folds the `SSAO_SLICES` rotated slices into the final `ao` factor — the top-level
/// SSAO body the [`crate::emit::emit_hlsl_ssao`] entry traces. Authored over `C`; the
/// hand-written shader supplies the center world position `P` (`p`), the center surface
/// normal `N` (`n`, the elevation reference — CONSTANT across all slices/taps), and the
/// forward-tap seam (`tap`, indexed by `(slice_iv, step_iv, sign)`).
///
/// The plan's `ssao_estimate`:
///   `occ = Σ_slices ssao_slice(...)`
///   `ao  = clamp01(1 - SSAO_STRENGTH * occ / SSAO_SLICES)`
///   `ao  = ao * ao`   (the integer self-mul strength power, NOT `pow`)
///
/// `tap(s, step, sign)` returns the forward-reconstructed neighbour `P'` for that slice's
/// step (the hand-written seam folds the slice's screen DIRECTION into the neighbour-pixel
/// pick — the horizon math no longer reads it). Returns the `ao` factor a caller stores
/// (the shader writes `ssao[px,py] = ao`).
#[inline]
pub fn ssao_estimate_body<C, T>(p: C::Vec3f, n: C::Vec3f, tap: &T) -> C::Scalar
where
    C: Cf,
    T: Fn(C::Iv, C::Iv, bool) -> C::Vec3f,
{
    // float occ = 0.0;  — the slice-occlusion accumulator.
    let occ = C::decl_var("occ", C::Scalar::lit(0.0));
    // The slice loop's `Flow` is discarded for the same reason as the step loops (no `ret`).
    let _ = C::runtime_for("[unroll]", "sl", "SSAO_SLICES", SSAO_SLICES, |s| -> Flow {
        // Bind the slice index into the per-step seam so `ssao_slice_body`'s `tap(step,
        // sign)` resolves the right slice's neighbour.
        let slice_tap = |step: C::Iv, sign: bool| tap(s, step, sign);
        C::set_var(
            &occ,
            C::get_var(&occ).add(ssao_slice_body::<C, _>(p, n, &slice_tap)),
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

    /// The center surface normal `N`, pointing toward the camera (+z out of the screen). The
    /// horizon math measures the neighbour's elevation above the tangent plane this normal
    /// defines (the x/y plane through the origin center).
    const CENTER_N: [f32; 3] = [0.0, 0.0, 1.0];

    /// Folds the estimate over a tiny synthetic neighbourhood. `tap(s, step, sign)` returns
    /// the neighbour world position `P'` for that tap; the center is the origin with normal
    /// `CENTER_N`. The closure builds the neighbour from the half-slice sign + step.
    fn estimate<T: Fn(usize, usize, bool) -> [f32; 3]>(tap: T) -> f32 {
        let p = [0.0f32, 0.0, 0.0];
        ssao_estimate_body::<EvalCf, _>(p, CENTER_N, &tap)
    }

    #[test]
    fn deep_crevice_neighbourhood_darkens_ao() {
        // A concave corner: every forward tap RISES toward the camera (along +N), so
        // `dot(delta, N) > 0` (a positive elevation above the tangent plane), well within the
        // SSAO_RADIUS falloff, so each tap's `elev * falloff` is large -> the horizon max is
        // high -> `occ` is large -> `ao` clearly < 1. Both half-slices see the same rising
        // wall (the screen offset moves in x/y, the surface lifts in +z).
        let tap = |_s: usize, step: usize, sign: bool| -> [f32; 3] {
            // March outward in pixel steps; the surface climbs toward the camera (+z = +N).
            let d = 0.06 * (step as f32 + 1.0); // < SSAO_RADIUS (0.5): falloff > 0
            let x = if sign { -d } else { d };
            // The neighbour is offset in-screen (x) AND lifted above the tangent (+z): the
            // crevice wall rising toward the camera.
            [x, 0.0, d]
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
        // A flat surface: every neighbour lies IN the tangent plane (`delta ⊥ N`, the x/y
        // plane with `N = +z`), so `dot(delta, N) == 0` -> `elev == 0` -> no horizon is
        // raised -> `occ == 0` -> `ao == 1`. This is the bug's regression guard: under the old
        // screen-direction math an in-plane neighbour parallel to the slice axis gave
        // `sampleCos ≈ 1` and BLACKENED a flat lit surface.
        let tap = |_s: usize, step: usize, sign: bool| -> [f32; 3] {
            let d = 0.06 * (step as f32 + 1.0);
            let x = if sign { -d } else { d };
            [x, 0.0, 0.0] // in the tangent plane (z == 0): zero elevation
        };
        let ao = estimate(tap);
        assert!(
            ao > 0.999,
            "a flat surface (delta perpendicular to N) must leave AO ~= 1.0, got ao = {ao}"
        );
    }

    #[test]
    fn no_neighbours_is_fully_unoccluded() {
        // Degenerate: every tap reconstructs `Pp == P` (the seam's out-of-bounds / non-lit
        // skip). `delta == 0` -> `elev == 0` (guarded by SSAO_EPS, no NaN) -> `ao == 1`.
        let ao = estimate(|_s, _step, _sign| [0.0, 0.0, 0.0]);
        assert!(
            (ao - 1.0).abs() < 1.0e-6,
            "an all-skipped neighbourhood must be fully unoccluded (ao = 1.0), got ao = {ao}"
        );
    }
}
