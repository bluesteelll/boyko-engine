//! The B1 over-relaxation ACCEPT-REFINE inner-loop leaf (Increment 4c: the production
//! main-marcher accept-refine `[loop]` SPAN re-expressed in the control eDSL).
//!
//! The B1 over-relaxation marcher (`sdf_gbuffer_composite.hlsl`'s main `for (uint it...)` at
//! L1328) sphere-traces the analytic field with an over-relaxation factor `omega`. On an
//! `abs(d) < EPS` accept (L1426), `d < EPS` is a ONE-SIDED upper bound: an over-relaxed step
//! (`omega > 1`) can jump from outside to DEEP inside in one stride, so the accepted `t` may sit
//! ~δ below the surface. Committing that `t` makes `sdf_soft_shadow` / `sdf_ao` sample from
//! inside the field → the surface renders BLACK. This module authors the SETTLE-ONTO-SURFACE
//! span — the committed `[loop] for (uint ri...) { ... t = t + step; }` of L1442-1452: a SIGNED,
//! under-relaxed sphere-trace (`t += M2_REFINE_RELAX * sdf(...)`) that walks BACKWARD for `d < 0`
//! (an overshot inside hit, back toward the surface) and forward for `d > 0`, accepting on
//! `abs(rd_) < EPS` — ONCE over the control-flow axis `C: Cf` + the field-call seam `field`. The
//! entire enclosing main marcher (the `hit = true; exhausted = false;` accept block, the
//! rationale comment, and the outer `break;` that exits the MARCHER loop) stays HAND-WRITTEN
//! inline in the committed shader (framing (b) — the generated text is spliced between the
//! `// === GENERATED b1_accept_refine BEGIN/END ===` sentinels, around which the marcher lives
//! un-generated).
//!
//! It is a near-CLONE of the proven [`crate::surface::m2_surface_hit_refine_body`], STRICTLY
//! SIMPLER: there is NO return facet at all (no bool/float return, no out-float, no composite
//! converged-hit combinator) — the only loop control is the [`Cf::brk`] on the `abs(rd_) < EPS`
//! accept. It reuses the SAME proven leaves: [`Cf::runtime_for`] (over the const bound symbol
//! `M2_REFINE_ITERS`), [`Cf::call1`] (`sdf(ro + rd * t)` — the interned `"sdf"` callee, NOT the
//! A1 hardcoded field-call vector node), [`Cf::brk`] (the `abs(rd_) < EPS` break), and
//! [`Cf::decl_param`] (the suppressed-decl carried `t`, declared by the enclosing marcher — same
//! as `m2_regula_falsi`'s `lo`/`hi`).
//!
//! Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — the CPU oracle (real `for`/`if`/`break`/`Cell` + the host `sdf` closure
//!     threaded as `field`, so [`Cf::call1`]'s `unreachable!` is never reached). The eval sweep
//!     reproduces the committed B1 accept-refine CONTROL FLOW (scoped to control flow — the
//!     cmp-`.spv` is the byte-identity oracle).
//!   - `<EmitCf>` — the HLSL recorder; the printer
//!     ([`crate::emit::emit_hlsl_b1_accept_refine`]) walks the STMT IR into the `[loop]` span
//!     (byte-identical to the committed `.comp.spv`, proven by the cmp-`.spv`).
//!
//! # The named tuning consts spell as SYMBOLS
//!
//! `M2_REFINE_RELAX` / `EPS` spell SYMBOLICALLY in the emitted HLSL (via [`Cf::named_lit`] bound
//! to the symbol), and `M2_REFINE_ITERS` is the `[loop]` bound symbol — a value-spelled const
//! would change the committed `OpConstant` set. The Eval values below mirror the committed shader
//! literals (`sdf_gbuffer_composite.hlsl:679,407,675`); they drive ONLY the Eval control-flow
//! oracle (the `.spv` gate is unaffected by the Eval value).
//!
//! # `R1` (no `+=` leaf)
//!
//! The step accumulation is `t = t + step` (the eDSL's natural [`Cf::set_var`] form), which the
//! GO/NO-GO spike proved compiles BYTE-IDENTICAL at this exact depth-3 site to the committed
//! `t += step` — so NO compound-assign leaf is added (the SAME R1 result the `sdf_soft_shadow`
//! and `m2_surface_hit` leaves proved).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The under-relaxation factor of the signed refine step (`step = M2_REFINE_RELAX * rd_`).
/// Mirrors the GPU's `M2_REFINE_RELAX` (`sdf_gbuffer_composite.hlsl:679`). Spelled SYMBOLICALLY
/// in the emitted HLSL (`M2_REFINE_RELAX`, NOT `0.8`).
pub const M2_REFINE_RELAX: f32 = 0.8;

/// The on-surface accept threshold on `abs(rd_)` (`if (abs(rd_) < EPS) { break; }`). Mirrors the
/// GPU's `EPS` (`sdf_gbuffer_composite.hlsl:407`). Spelled SYMBOLICALLY (`EPS`, NOT `0.001`).
pub const EPS: f32 = 0.001;

/// The fixed refine budget — the accept-refine `[loop]` trip count, the BOUND SYMBOL the
/// for-header carries. Mirrors the GPU's `M2_REFINE_ITERS` (`sdf_gbuffer_composite.hlsl:675`).
/// Spelled SYMBOLICALLY in the emitted HLSL header (`M2_REFINE_ITERS`, NOT `8u`).
pub const M2_REFINE_ITERS: usize = 32;

/// Settles the B1 over-relaxation ACCEPT `t` onto the EXACT surface by a SIGNED, under-relaxed
/// sphere-trace, mutating the carried `t` in place and returning its final value. Authored ONCE
/// over the control-flow axis `C` plus the field-call seam `field`. Mirrors the GPU B1 marcher's
/// L1442-1452 accept-refine `[loop]` span statement-for-statement (the enclosing `hit = true;
/// exhausted = false;` accept block + the outer marcher `break;` stay hand-written inline).
///
/// `ro` / `rd` are the world ray origin/direction; `t_seed` the candidate world `t` the enclosing
/// marcher carries in (declared OUTSIDE the span — a [`Cf::decl_param`] suppressed-decl, like
/// `m2_regula_falsi`'s `lo`/`hi`). `field` is the field-distance seam: on Eval it is the host
/// `sdf` closure (so `b1_accept_refine_body::<EvalCf>` re-runs the host field at each `ro + rd *
/// t`); on Emit it records a `sdf(ro + rd * t)` call node (via [`Cf::call1`] with the interned
/// `"sdf"` callee).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; Continue }`. There is NO function
/// return facet (the simplest 4b-family span): the only loop control is the `abs(rd_) < EPS`
/// [`Cf::brk`], propagated through [`Cf::if_`]'s `?`; [`Cf::runtime_for`] CONSUMES it (the loop
/// returns `Continue`), so the IIFE falls through and the carried `t` holds the settled value. On
/// Emit every branch is recorded structurally (`?` never early-returns); the whole span is
/// captured. The returned `C::get_var(&t)` is the observed result on Eval; on Emit it is the `t`
/// variable handle, discarded by the producer (the span mutates `t` by name).
#[inline]
pub fn b1_accept_refine_body<C: Cf, F: Fn(C::Vec3f) -> C::Scalar>(
    ro: C::Vec3f,
    rd: C::Vec3f,
    t_seed: C::Scalar,
    field: F,
) -> C::Scalar {
    // float t = ...;  (the carried var the enclosing marcher declares — a suppressed-decl param,
    // so get/set spell `t`/`t = ...;` but NO `float t = ...;` redecl is recorded inside the span).
    let t = C::decl_param("t", t_seed);

    let run = || -> Flow {
        C::runtime_for("[loop]", "ri", "M2_REFINE_ITERS", M2_REFINE_ITERS, |_ri| -> Flow {
            // float rd_ = sdf(ro + rd * t);  — a NAMED `float` temp via the threaded `field` seam
            // (matching the committed materialization; the committed name is `rd_`).
            let rd_ = C::temp_float(
                "rd_",
                field(C::vec3_add(ro, C::vec3_mul_scalar(rd, C::get_var(&t)))),
            );
            // if (abs(rd_) < EPS) { break; }  — the on-surface accept; `if_`'s `?` propagates the
            // `brk` token, which runtime_for CONSUMES (the loop ends; the IIFE falls through).
            C::if_(rd_.abs().lt(C::named_lit("EPS", EPS)), C::brk)?;
            // float step = M2_REFINE_RELAX * rd_;  — a NAMED `float` temp (PINS the multiply as
            // its own rounding, so `t = t + step` rounds bit-identically to the committed two-
            // rounding `step = M2_REFINE_RELAX * rd_; t += step;`; no FMA contraction at O3).
            let relax = C::named_lit("M2_REFINE_RELAX", M2_REFINE_RELAX);
            let step = C::temp_float("step", relax.mul(rd_));
            // t = t + step;  (R1: byte-identical to the committed `t += step` at this depth-3 site
            // — the spike proved it; no compound-assign leaf).
            C::set_var(&t, C::get_var(&t).add(step));
            Flow::Continue(())
        })?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the loop consumed its own break; on Emit the recorder captured
    // every statement.
    let _ = run();

    // The settled `t` — the observed Eval result; on Emit the `t` handle, discarded by the producer.
    C::get_var(&t)
}
