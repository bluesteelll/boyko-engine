//! The SDF cone-trace soft-shadow marcher leaf (Increment 4b: the FIRST marcher
//! `[loop]` re-expressed in the control eDSL).
//!
//! `sdf_soft_shadow` (`sdf_gbuffer_composite.hlsl:450-469`) is the SMALLEST genuine
//! marcher runtime `[loop]` — a fixed-budget (`MAX_IT`) penumbra-min march that calls the
//! FROZEN `field_distance` once per step, tracks the min penumbra ratio `SHADOW_K * d / t`,
//! EARLY-RETURNS `0.0` on an occluder hit (`d < SHADOW_HIT_EPS`), and BREAKS when the ray
//! escapes the bound (`t > T_MAX`), returning `clamp(res, 0.0, 1.0)`. This module authors the
//! `[loop]`+tail SPAN (the `float res; float t; [loop] {...} return clamp(...)` of L454-468)
//! ONCE over the control-flow axis `C: Cf` + the field-call seam; the `dot(n, L)` early-return
//! PREAMBLE (L451-453) stays HAND-WRITTEN inline in the committed shader (framing (b) — the
//! generated text is spliced between the `// === GENERATED sdf_soft_shadow BEGIN/END ===`
//! sentinels INSIDE `sdf_soft_shadow`, above which the preamble lives un-generated).
//!
//! It STAGES the two new leaves on the proven Inc-4a `runtime_for` foundation:
//! - [`Cf::brk`] — the loop BREAK producer (the consumer arms in `runtime_for`/`unroll_for`
//!   already exist + are tested; this is the matching producer, `if (t > T_MAX) { break; }`);
//! - [`Cf::call1`] — the single-`float3`-arg `float`-returning call site (`field_distance(p +
//!   L * t)`), the float3→float analogue of `call2`.
//!
//! Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — the CPU oracle (real `for`/`if`/`break`/`Cell` + the host `field_distance`
//!     closure threaded as `field`, so `Cf::call1`'s `unreachable!` is never reached). The
//!     eval sweep reproduces the host `sdf_soft_shadow` CONTROL FLOW (scoped to control flow —
//!     the cmp-`.spv` is the byte-identity oracle).
//!   - `<EmitCf>` — the HLSL recorder; the printer
//!     (`crate::emit::emit_hlsl_sdf_soft_shadow`) walks the STMT IR into the `[loop]`+tail
//!     span (byte-identical to the committed `.comp.spv`, proven by the cmp-`.spv`).
//!
//! # The named tuning consts spell as SYMBOLS
//!
//! `SHADOW_MINT` / `SHADOW_K` / `SHADOW_HIT_EPS` / `FIELD_LIPSCHITZ_L` / `SHADOW_MINT_STEP` /
//! `T_MAX` spell SYMBOLICALLY in the emitted HLSL (via [`Cf::named_lit`] / [`crate::scalar::
//! FieldScalar::lit`] bound to the symbol), and `MAX_IT` is the `[loop]` bound symbol — a
//! value-spelled const would change the committed `OpConstant` set. The Eval values below
//! mirror the committed shader literals (`sdf_gbuffer_composite.hlsl:407-436` +
//! `sdf_field.hlsli:42,259`); they drive ONLY the Eval control-flow oracle (the `.spv` gate
//! is unaffected by the Eval value).
//!
//! # `R1` (no `+=` leaf)
//!
//! The step accumulation is `t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP)` (the
//! eDSL's natural [`Cf::set_var`] form), which the GO/NO-GO spike proved compiles
//! BYTE-IDENTICAL to the committed `t += max(...)` — so NO compound-assign leaf is added.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The penumbra hardness — `sdf_soft_shadow`'s `SHADOW_K * d / t` cone factor. Mirrors the
/// GPU's `SHADOW_K` (`sdf_gbuffer_composite.hlsl:433`) and the host owner-default. Spelled
/// SYMBOLICALLY in the emitted HLSL (`SHADOW_K`, NOT `8.0`).
pub const SHADOW_K: f32 = 8.0;

/// The march start offset (`float t = SHADOW_MINT;`). The committed shader defines it as
/// `16.0 * GRAD_H` (`sdf_gbuffer_composite.hlsl:434`, `GRAD_H = 0.0005`); the Eval value is
/// the product `0.008`. Spelled SYMBOLICALLY in the emitted HLSL (`SHADOW_MINT`).
pub const SHADOW_MINT: f32 = 16.0 * 0.0005;

/// The minimum per-step advance (the floor on `d / FIELD_LIPSCHITZ_L`) — `16.0 * GRAD_H`
/// (`sdf_gbuffer_composite.hlsl:435`), Eval value `0.008`. Spelled SYMBOLICALLY
/// (`SHADOW_MINT_STEP`).
pub const SHADOW_MINT_STEP: f32 = 16.0 * 0.0005;

/// The occluder-hit threshold (`if (d < SHADOW_HIT_EPS) { return 0.0; }`) — `2.0 * EPS`
/// (`sdf_gbuffer_composite.hlsl:436`, `EPS = 0.001`), Eval value `0.002`. Spelled
/// SYMBOLICALLY (`SHADOW_HIT_EPS`).
pub const SHADOW_HIT_EPS: f32 = 2.0 * 0.001;

/// The cone step's distance divisor — the k-independent worst-case spatial gradient magnitude
/// (`sqrt(2)`). Mirrors the GPU's `FIELD_LIPSCHITZ_L` (`sdf_field.hlsli:259`). Spelled
/// SYMBOLICALLY (`FIELD_LIPSCHITZ_L`).
///
/// The literal is the COMMITTED shader's `1.41421356` VERBATIM (the single-source discipline
/// the other GPU-shape consts use, e.g. `M2_REGULA_DENOM_EPS`) rather than
/// `core::f32::consts::SQRT_2` — the Eval value must SPELL the committed GPU literal so a reader
/// diffing this file against `sdf_field.hlsli` sees one token, not two spellings of one number.
///
/// ⚠️ The reason is provenance, NOT arithmetic. An earlier revision of this doc claimed
/// "`SQRT_2`'s f32 is a DIFFERENT bit pattern"; it is not — MEASURED, `1.41421356f32` and
/// `core::f32::consts::SQRT_2` are both `0x3fb504f3`, so substituting one for the other would
/// change no result of this oracle. In a module whose whole subject is bit-exactness a wrong
/// bit-pattern claim is the kind of thing a later reader builds on, so it is corrected here
/// instead of deleted. The lints (`approx_constant` suggests `SQRT_2`; `excessive_precision`
/// notes the extra digits) are suppressed because the spelling is deliberate.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub const FIELD_LIPSCHITZ_L: f32 = 1.41421356;

/// The miss-distance bound (`if (t > T_MAX) { break; }`). Mirrors the GPU's `T_MAX`
/// (`sdf_gbuffer_composite.hlsl:408`). Spelled SYMBOLICALLY (`T_MAX`, NOT `10.0`).
pub const T_MAX: f32 = 10.0;

/// The max march steps per ray — `sdf_soft_shadow`'s `[loop]` trip count, the BOUND SYMBOL
/// the for-header carries. Mirrors the GPU's `MAX_IT` (`sdf_gbuffer_composite.hlsl:409`).
/// Spelled SYMBOLICALLY in the emitted HLSL header (`MAX_IT`, NOT `128u`).
pub const MAX_IT: usize = 128;

/// Marches the SDF cone-trace soft shadow from `p` toward the light `L`, depositing the
/// visibility `clamp(res, 0.0, 1.0)` into `out`. Authored ONCE over the control-flow axis `C`
/// plus the field-call seam `field`. Mirrors the GPU `sdf_soft_shadow`'s L454-468 LOOP+TAIL
/// span statement-for-statement (the `dot(n, L)` early-return preamble stays hand-written
/// inline).
///
/// `p` is the surface point, `n` the surface normal (consumed ONLY by the hand-written
/// preamble — UNUSED in this generated span, carried for the body's signature parity), `L` the
/// (normalized) light direction. `field` is the field-distance seam (see [`crate::normal`]'s
/// field-call seam): on Eval it is the host `field_distance` closure (so
/// `sdf_soft_shadow_body::<EvalCf>` re-runs the host field at each `p + L * t`); on Emit it
/// records a `field_distance(p + L * t)` call node (via [`Cf::call1`]).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; ret_f(out, clamp01)?; Continue }`,
/// so an in-loop [`Cf::if_ret_f`]'s `Break(Return)` (the `d < SHADOW_HIT_EPS` occluder hit)
/// forwards through [`Cf::runtime_for`]'s `?` to the IIFE's `?` — skipping the tail (the early
/// `0.0` is the result). The `t > T_MAX` break is a [`Cf::brk`] propagated through
/// [`Cf::if_`]'s `?`; `runtime_for` CONSUMES it (the loop returns `Continue`), so the post-loop
/// `clamp(res, 0.0, 1.0)` tail RUNS. On Emit every branch is recorded structurally (`?` never
/// early-returns); the whole span is captured.
#[inline]
pub fn sdf_soft_shadow_body<C: Cf, F: Fn(C::Vec3f) -> C::Scalar>(
    p: C::Vec3f,
    _n: C::Vec3f,
    l: C::Vec3f,
    field: F,
    out: &C::RetCellF,
) {
    let run = || -> Flow {
        // float res = 1.0;  /  float t = SHADOW_MINT;  (both TRUE locals — recorded DeclVars).
        let res = C::decl_var("res", C::Scalar::lit(1.0));
        let t = C::decl_var("t", C::named_lit("SHADOW_MINT", SHADOW_MINT));

        // Inc-4a carry-in #2 (nested iv collision) NOT exercised: single non-indexing loop
        // (the body never spells `i`, so the per-loop iv-id fix is not load-bearing here).
        C::runtime_for("[loop]", "i", "MAX_IT", MAX_IT, |_i| -> Flow {
            // float d = field_distance(p + L * t);
            let d = C::temp_float(
                "d",
                field(C::vec3_add(p, C::vec3_mul_scalar(l, C::get_var(&t)))),
            );
            // res = min(res, SHADOW_K * d / t);  — `min(res, ((K*d)/t))` (left-to-right
            // `k.mul(d).div(t)`, printing `SHADOW_K * d / t` un-parenthesized).
            let k = C::named_lit("SHADOW_K", SHADOW_K);
            C::set_var(&res, C::get_var(&res).min(k.mul(d).div(C::get_var(&t))));
            // if (d < SHADOW_HIT_EPS) { return 0.0; }  — the in-loop occluder-hit early return
            // (forwarded through runtime_for to the IIFE by `?`).
            let hit_eps = C::named_lit("SHADOW_HIT_EPS", SHADOW_HIT_EPS);
            C::if_ret_f(out, d.lt(hit_eps), C::Scalar::lit(0.0))?;
            // t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);  (R1: byte-identical to
            // the committed `t += max(...)` — no compound-assign leaf).
            let lip = C::named_lit("FIELD_LIPSCHITZ_L", FIELD_LIPSCHITZ_L);
            let step = C::named_lit("SHADOW_MINT_STEP", SHADOW_MINT_STEP);
            C::set_var(&t, C::get_var(&t).add(d.div(lip).max(step)));
            // if (t > T_MAX) { break; }  — the escape break; `if_`'s `?` propagates the `brk`
            // token, which runtime_for CONSUMES (the post-loop tail then runs).
            let tmax = C::named_lit("T_MAX", T_MAX);
            C::if_(C::get_var(&t).gt(tmax), C::brk)?;
            Flow::Continue(())
        })?;
        // return clamp(res, 0.0, 1.0);  (reached when the loop exhausts its budget or breaks).
        C::ret_f(out, C::get_var(&res).clamp01())?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the early `?` already deposited the visibility into `out`; on
    // Emit the recorder captured every statement.
    let _ = run();
}

/// The `t_max`-RANGED clone of [`sdf_soft_shadow_body`] (P6 R1 — multi-light SDF shadows).
///
/// Statement-for-statement IDENTICAL to [`sdf_soft_shadow_body`] EXCEPT the escape break
/// bound: the hardcoded `T_MAX` symbol is replaced by the runtime parameter `t_max` (the
/// `if (t > t_max) { break; }`). Authored as a SEPARATE entrypoint (B3 — option a) so the
/// marcher's frozen `sdf_soft_shadow` emit / `.comp.spv` cannot move: this generator is
/// consumed ONLY by the RESOLVE (`deferred_pbr.hlsl`), never the marcher.
///
/// `t_max` is the per-caster march bound the resolve passes: the light DISTANCE for a
/// punctual (point/spot) caster (the common, cheap, nearby case) or `T_MAX` for an extra
/// directional caster. `p`/`_n`/`l`/`field`/`out` carry the SAME contract as
/// [`sdf_soft_shadow_body`] (the `dot(n, L)` early-return preamble stays hand-written inline
/// in the RESOLVE's per-light loop — the generated span is the loop+tail only).
#[inline]
pub fn sdf_soft_shadow_ranged_body<C: Cf, F: Fn(C::Vec3f) -> C::Scalar>(
    p: C::Vec3f,
    _n: C::Vec3f,
    l: C::Vec3f,
    t_max: C::Scalar,
    field: F,
    out: &C::RetCellF,
) {
    let run = || -> Flow {
        // float res = 1.0;  /  float t = SHADOW_MINT;  (both TRUE locals — recorded DeclVars).
        let res = C::decl_var("res", C::Scalar::lit(1.0));
        let t = C::decl_var("t", C::named_lit("SHADOW_MINT", SHADOW_MINT));

        C::runtime_for("[loop]", "i", "MAX_IT", MAX_IT, |_i| -> Flow {
            // float d = field_distance(p + L * t);
            let d = C::temp_float(
                "d",
                field(C::vec3_add(p, C::vec3_mul_scalar(l, C::get_var(&t)))),
            );
            // res = min(res, SHADOW_K * d / t);
            let k = C::named_lit("SHADOW_K", SHADOW_K);
            C::set_var(&res, C::get_var(&res).min(k.mul(d).div(C::get_var(&t))));
            // if (d < SHADOW_HIT_EPS) { return 0.0; }
            let hit_eps = C::named_lit("SHADOW_HIT_EPS", SHADOW_HIT_EPS);
            C::if_ret_f(out, d.lt(hit_eps), C::Scalar::lit(0.0))?;
            // t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);
            let lip = C::named_lit("FIELD_LIPSCHITZ_L", FIELD_LIPSCHITZ_L);
            let step = C::named_lit("SHADOW_MINT_STEP", SHADOW_MINT_STEP);
            C::set_var(&t, C::get_var(&t).add(d.div(lip).max(step)));
            // if (t > t_max) { break; }  — THE ONLY DIVERGENCE from `sdf_soft_shadow_body`:
            // the RUNTIME parameter `t_max` replaces the frozen `T_MAX` symbol.
            C::if_(C::get_var(&t).gt(t_max), C::brk)?;
            Flow::Continue(())
        })?;
        // return clamp(res, 0.0, 1.0);
        C::ret_f(out, C::get_var(&res).clamp01())?;
        Flow::Continue(())
    };
    let _ = run();
}
