//! The M2 brick-marcher SURFACE-HIT refine leaf (Increment 4b.2: the production
//! `m2_surface_hit` analytic-residual refine loop+tail SPAN re-expressed in the control eDSL).
//!
//! `m2_surface_hit` (`sdf_gbuffer_composite.hlsl:1104`) is the production brick-atlas
//! surface-hit decider: after the integer cell-addressing preamble + the `m2_brick_span` /
//! `m2_brick_cubic_hit` call sites land a cubic candidate `cand_t`, a SIGNED, under-relaxed
//! sphere-trace (the analytic-residual fallback) refines that candidate onto the EXACT field,
//! converging from EITHER side, and decides hit (`return true` + the refined `hit_t`) vs miss
//! (`return false`, the caller folds the M1 analytic field). This module authors the
//! `[loop]`+tail SPAN (the committed `float rt = cand_t; [loop] {...} return false;` of
//! L1184-1205) ONCE over the control-flow axis `C: Cf` + the field-call seam; the integer
//! cell-addressing PREAMBLE (the 7-param header, the `hit_t = t_world;` entry default, the
//! field-unpacks, the rel/tile float-guard early returns, the M5 toroidal-slot INTEGER math,
//! and the `m2_brick_span` / `m2_brick_cubic_hit` / `select_level` CALL SITES) stays
//! HAND-WRITTEN inline in the committed shader (framing (b) — the generated text is spliced
//! between the `// === GENERATED m2_surface_hit_refine BEGIN/END ===` sentinels INSIDE
//! `m2_surface_hit`, above/around which the preamble lives un-generated).
//!
//! It STAGES on the proven Inc-4b foundation, reusing [`Cf::brk`] (the `rt < 0 || rt > T_MAX`
//! break), [`Cf::call1`] (`field_distance(ro + rd * rt)`), [`Cf::runtime_for`] (over the const
//! bound symbol `M2_REFINE_ITERS`), and adding two tiny facets:
//! - the BOOL RETURN ([`Cf::ret_b`]) — a real `OpTypeBool` return (`return true`/`return
//!   false`, the binary fact the spike read off the committed `.comp.spv`: `OpConstantTrue` /
//!   `OpConstantFalse`, NOT `uint` 0/1);
//! - the OUT-FLOAT ([`Cf::out_float_assign`]) — the in-loop `hit_t = rt;` write into the `out
//!   float hit_t` parameter (the entry `hit_t = t_world;` default stays hand-written);
//!
//! folded together by the composite combinator [`Cf::if_hit_ret_b`] (records `hit_t = rt;` THEN
//! `return true;` in ONE then-block, so on Eval `hit_t` is written BEFORE the `Break(Return)`
//! short-circuits — the oracle reads the FRESH `rt`).
//!
//! Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — the CPU oracle (real `for`/`if`/`break`/`Cell` + the host `field_distance`
//!     closure threaded as `field`, so [`Cf::call1`]'s `unreachable!` is never reached). The
//!     eval sweep reproduces the host `host_m2_surface_hit` refine CONTROL FLOW (scoped to
//!     control flow — the cmp-`.spv` is the byte-identity oracle).
//!   - `<EmitCf>` — the HLSL recorder; the printer
//!     ([`crate::emit::emit_hlsl_m2_surface_hit_refine`]) walks the STMT IR into the
//!     `[loop]`+tail span (byte-identical to the committed `.comp.spv`, proven by the cmp-`.spv`).
//!
//! # The named tuning consts spell as SYMBOLS
//!
//! `M2_REFINE_RELAX` / `EPS` / `T_MAX` spell SYMBOLICALLY in the emitted HLSL (via
//! [`Cf::named_lit`] bound to the symbol), and `M2_REFINE_ITERS` is the `[loop]` bound symbol —
//! a value-spelled const would change the committed `OpConstant` set. The Eval values below
//! mirror the committed shader literals (`sdf_gbuffer_composite.hlsl:407,675,679`); they drive
//! ONLY the Eval control-flow oracle (the `.spv` gate is unaffected by the Eval value).
//!
//! # `R1` (no `+=` leaf)
//!
//! The step accumulation is `rt = rt + step` (the eDSL's natural [`Cf::set_var`] form), which
//! the GO/NO-GO spike proved compiles BYTE-IDENTICAL to the committed `rt += step` — so NO
//! compound-assign leaf is added (the SAME R1 result the `sdf_soft_shadow` leaf proved).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The under-relaxation factor of the signed refine step (`step = M2_REFINE_RELAX * d`).
/// Mirrors the GPU's `M2_REFINE_RELAX` (`sdf_gbuffer_composite.hlsl:679`). Spelled
/// SYMBOLICALLY in the emitted HLSL (`M2_REFINE_RELAX`, NOT `0.8`).
pub const M2_REFINE_RELAX: f32 = 0.8;

/// The on-surface hit threshold on `abs(d)` (`if (abs(d) < EPS) { ... }`). Mirrors the GPU's
/// `EPS` (`sdf_gbuffer_composite.hlsl:407`). Spelled SYMBOLICALLY (`EPS`, NOT `0.001`).
pub const EPS: f32 = 0.001;

/// The miss-distance bound (`if (rt < 0.0 || rt > T_MAX) { break; }`). Mirrors the GPU's
/// `T_MAX` (`sdf_gbuffer_composite.hlsl:408`). Spelled SYMBOLICALLY (`T_MAX`, NOT `10.0`).
pub const T_MAX: f32 = 10.0;

/// The fixed refine budget — `m2_surface_hit`'s `[loop]` trip count, the BOUND SYMBOL the
/// for-header carries. Mirrors the GPU's `M2_REFINE_ITERS` (`sdf_gbuffer_composite.hlsl:675`).
/// Spelled SYMBOLICALLY in the emitted HLSL header (`M2_REFINE_ITERS`, NOT `8u`).
pub const M2_REFINE_ITERS: usize = 32;

/// Refines the cubic candidate `cand_t` onto the EXACT field by a SIGNED, under-relaxed
/// sphere-trace, depositing the refined hit `t` into `hit_out` and returning `true` (converged,
/// `abs(d) < EPS`) or `false` (budget exhausted / escaped the bound) into `ret_out`. Authored
/// ONCE over the control-flow axis `C` plus the field-call seam `field`. Mirrors the GPU
/// `m2_surface_hit`'s L1184-1205 LOOP+TAIL span statement-for-statement (the integer
/// cell-addressing preamble + the entry `hit_t = t_world;` default stay hand-written inline).
///
/// `ro` / `rd` are the world ray origin/direction; `cand_t` the cubic candidate world `t` (the
/// refine's starting point). `field` is the field-distance seam (see [`crate::shadow`]'s
/// field-call seam): on Eval it is the host `field_distance` closure (so
/// `m2_surface_hit_refine_body::<EvalCf>` re-runs the host field at each `ro + rd * rt`); on
/// Emit it records a `field_distance(ro + rd * rt)` call node (via [`Cf::call1`]).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; ret_b(ret_out, false)?; Continue }`,
/// so an in-loop [`Cf::if_hit_ret_b`]'s `Break(Return)` (the `abs(d) < EPS` converged hit)
/// forwards through [`Cf::runtime_for`]'s `?` to the IIFE's `?` — skipping the tail (the early
/// `true` + the fresh `hit_t` is the result). The `rt < 0 || rt > T_MAX` escape is a
/// [`Cf::brk`] propagated through [`Cf::if_`]'s `?`; `runtime_for` CONSUMES it (the loop returns
/// `Continue`), so the post-loop `return false;` tail RUNS. On Emit every branch is recorded
/// structurally (`?` never early-returns); the whole span is captured.
#[inline]
pub fn m2_surface_hit_refine_body<C: Cf, F: Fn(C::Vec3f) -> C::Scalar>(
    ro: C::Vec3f,
    rd: C::Vec3f,
    cand_t: C::Scalar,
    field: F,
    hit_out: &C::OutFloat,
    ret_out: &C::RetCellB,
) {
    let run = || -> Flow {
        // float rt = cand_t;  (a TRUE local — carry-in #1 NOT forced; Float default fits).
        let rt = C::decl_var("rt", cand_t);

        C::runtime_for("[loop]", "i", "M2_REFINE_ITERS", M2_REFINE_ITERS, |_i| -> Flow {
            // float d = field_distance(ro + rd * rt);  — a NAMED `float` temp (blocks any FMA
            // contraction; the committed two-rounding discipline) via the threaded `field` seam.
            let d = C::temp_float(
                "d",
                field(C::vec3_add(ro, C::vec3_mul_scalar(rd, C::get_var(&rt)))),
            );
            // if (abs(d) < EPS) { hit_t = rt; return true; }  — the composite in-loop hit: BOTH
            // `hit_t = rt;` (out_float_assign) AND `return true;` (ret_b) in ONE then-block, so
            // on Eval `hit_t` is written BEFORE the `Break(Return)` short-circuits (the oracle
            // reads the FRESH `rt`). The `?` forwards the return through `runtime_for` to the IIFE.
            let eps = C::named_lit("EPS", EPS);
            C::if_hit_ret_b(hit_out, ret_out, d.abs().lt(eps), C::get_var(&rt))?;
            // float step = M2_REFINE_RELAX * d;  — a NAMED `float` temp (PINS the multiply as its
            // own rounding, so `rt = rt + step` rounds bit-identically to the committed two-
            // rounding `step = M2_REFINE_RELAX * d; rt += step;`; no FMA contraction at O3).
            let relax = C::named_lit("M2_REFINE_RELAX", M2_REFINE_RELAX);
            let step = C::temp_float("step", relax.mul(d));
            // rt = rt + step;  (R1: byte-identical to the committed `rt += step` — no
            // compound-assign leaf, the same R1 the soft-shadow leaf proved).
            C::set_var(&rt, C::get_var(&rt).add(step));
            // if (rt < 0.0 || rt > T_MAX) { break; }  — the escape break; `if_`'s `?` propagates
            // the `brk` token, which `runtime_for` CONSUMES (the post-loop tail then runs).
            let tmax = C::named_lit("T_MAX", T_MAX);
            let escaped = C::or(C::get_var(&rt).lt(C::Scalar::lit(0.0)), C::get_var(&rt).gt(tmax));
            C::if_(escaped, C::brk)?;
            Flow::Continue(())
        })?;
        // return false;  (reached when the refine budget exhausts or the ray escapes the bound —
        // no confident hit in this brick; the caller folds the M1 analytic field).
        C::ret_b(ret_out, false)?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the early `?` already deposited hit_t + true into the cells; on
    // Emit the recorder captured every statement.
    let _ = run();
}
