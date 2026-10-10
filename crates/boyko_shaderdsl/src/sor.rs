//! The B1 over-relaxation SOR-FAIL-RETREAT SPAN leaf (Increment 4f — the DEFENSIBLE TERMINUS of
//! the B1-marcher single-source ladder: the Keinert over-relaxation step + the Lipschitz-aware
//! retreat-to-plain, re-expressed in the control eDSL).
//!
//! # The span
//!
//! The B1 over-relaxation marcher (`sdf_gbuffer_composite.hlsl`'s main `for (uint it...)`)
//! sphere-traces the analytic field with an over-relaxation factor `omega > 1`. This module authors
//! the per-iteration STEP span — the committed L1459-1498:
//!
//! ```hlsl
//! if (omega > 1.0) {
//!     float step_len = d * omega;
//!     if (it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev) {
//!         t = safe_t + sor_prev;
//!         omega = 1.0;
//!         continue;
//!     }
//!     safe_t = t;
//!     sor_prev = d;
//!     sor_step_prev = step_len;
//!     t += step_len;
//! } else {
//!     t += d;
//! }
//! if (t > T_MAX) {
//!     exhausted = false;
//!     break;
//! }
//! ```
//!
//! The HAND-WRITTEN frame (out of this span) is: the `[loop] for (uint it...)` header, the
//! `t >= t_mesh` mesh-guard, the M1/M2 brick islands, `float d = sdf(p);`, the `d < EPS` accept
//! block (with its own Inc4c `b1_accept_refine` sentinel) — the generated text is spliced between
//! the `// === GENERATED b1_sor_retreat BEGIN/END ===` sentinels (framing (b)). The span prints at
//! DEPTH 2 (the `if (omega > 1.0)` 8-space indent — the site nests main→`for (uint it)`→this span).
//! Note: this is the SECOND generated sentinel inside the one hand-written `for (uint it)` loop
//! (Inc4c's accept-refine at depth 3 + this at depth 2 — both spliced into the same loop body).
//!
//! # BUG-B1-HOLE-1 rationale — the Lipschitz-aware retreat threshold (the WHY travels with the
//! source-of-truth)
//!
//! `sor_fail`: the over-step taken last iter overshot the previous unbounding sphere. Valid only for
//! `omega < 2` (host-clamped). Spheres must overlap or we may have skipped a surface. The threshold
//! is LIPSCHITZ-AWARE: IQ's smooth-min is super-Lipschitz, so the guaranteed-empty radius at field
//! value `f` is `f / L`, not `f`. Two empty balls (radii `sor_prev / L`, `d / L`) cover the
//! over-relaxed step `sor_step_prev` iff `sor_prev + d >= L * sor_step_prev`; the retreat must fire
//! when that fails. Multiplying the threshold by `FIELD_LIPSCHITZ_L` keeps the lower-bound invariant
//! sound in blend bands.
//!
//! The `it > 0u` guard is LOAD-BEARING (do not remove): a sor-fail can only be reached after at
//! least one ACCEPTED over-relax step (`it >= 1` ⟹ accepted >= 1), which pre-pays the +1 retreat
//! iteration in the budget proof below.
//!
//! # BUG-B1-HOLE-2 rationale — the plain-resume retreat (not a bare-`safe_t` re-probe)
//!
//! Do NOT retreat to bare `safe_t` and re-probe. That re-evals the field at `safe_t` (costing +2
//! iters vs a plain march), and on a ray converging at the `MAX_IT` cliff the extra probe overflows
//! the budget → a missed-surface hole. Instead RESUME the plain march ONE certified step past the
//! safe point: `safe_t` is the exact probe param and `sor_prev` is the exact field value sampled
//! there, so `safe_t + sor_prev` is precisely where a plain march lands after probing `safe_t` —
//! reusing that eval (no re-probe). The add is one same-sign FMA-free addition (both operands >= 0):
//! no catastrophic cancellation, unlike a `t - <correction>` subtraction form. Net cost is +1
//! iteration vs plain, pre-paid by the >= 1 accepted over-step (the `it > 0` guard). After the
//! retreat, `omega = 1.0` permanently falls to plain for the rest of this ray.
//!
//! # The 0%-gate
//!
//! The ENTIRE over-relaxation block is gated behind `if (omega > 1.0)`; the else-arm is the VERBATIM
//! frozen `t += d`, so at `omega == 1.0` the live path is textually the pre-B1 plain sphere-trace.
//! The frozen ordering (the over-relax step, then the `t > T_MAX` miss test) is preserved exactly.
//!
//! # Instantiation (the established control-axis discipline)
//!
//! - `<EvalCf>` — the CPU oracle (real `if`/`else`/`continue`/`break`/`Cell`). The eval sweep
//!   reproduces the committed sor-retreat CONTROL FLOW (scoped to control flow — the cmp-`.spv` is
//!   the byte-identity oracle).
//! - `<EmitCf>` — the HLSL recorder; the printer (`crate::emit::emit_hlsl_b1_sor_retreat`) walks
//!   the STMT IR into the span (byte-identical to the committed `.comp.spv`, proven by the cmp-`.spv`).
//!
//! # `it` is a CAPTURED `uint` local (NO iv-id refactor)
//!
//! The `for (uint it...)` loop is HAND-WRITTEN, so inside this span `it` is a CAPTURED outer-scope
//! `uint` read as a bare identifier (the `uint` analogue of how Inc4e read the captured `float`
//! `t_mesh`). The body takes `it: C::Uint` as a parameter; the producer seeds `Emit::uint_input(0)`
//! and passes `uint_in = ["it"]` (vs the `NO_UINT_INPUTS` most spans pass).
//!
//! # The named tuning consts spell as SYMBOLS
//!
//! `FIELD_LIPSCHITZ_L` / `T_MAX` spell SYMBOLICALLY in the emitted HLSL (via [`Cf::named_lit`]); a
//! value-spelled const would change the committed `OpConstant` set. The Eval values below mirror the
//! committed shader literals; they drive ONLY the Eval control-flow oracle (the `.spv` gate is
//! unaffected by the Eval value).
//!
//! # FMA pin
//!
//! `FIELD_LIPSCHITZ_L * sor_step_prev` is an INLINE `Mul` (NOT a `temp_float`) inside the `<`, while
//! `d * omega` IS the named `temp_float` `step_len`. This matches the committed materialization
//! (`step_len` is a named local; the threshold product is inline in the condition). The cmp-`.spv`
//! gates whether DXC -O0 contracts the inline product differently (expected: no contraction).
//!
//! # `R1` (no `+=` leaf)
//!
//! Both step accumulations are `t = t + <step>` (the eDSL's natural [`Cf::set_var`] form). The
//! GO/NO-GO R1 spike proved BOTH `t += step_len` (the omega arm) and the omega-else `t += d` rewrite
//! to `t = t + …` BYTE-IDENTICAL at this depth-2 site — so NO compound-assign leaf is added (the
//! SAME R1 result the prior B1 rungs proved).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The cone step's distance divisor — the k-INDEPENDENT worst-case spatial gradient magnitude of
/// IQ's smooth-min (`sqrt(2)`). Mirrors the GPU's `FIELD_LIPSCHITZ_L` (`sdf_field.hlsli:259`).
/// Spelled SYMBOLICALLY in the emitted HLSL (`FIELD_LIPSCHITZ_L`, NOT `1.41421356`).
///
/// The literal is the COMMITTED shader's `1.41421356` VERBATIM (the single-source discipline the
/// other GPU-shape consts use, IDENTICAL to `shadow::FIELD_LIPSCHITZ_L`), NOT
/// `core::f32::consts::SQRT_2`: the Eval value must spell the committed GPU literal so the Eval
/// control-flow oracle mirrors the shader's march on each sample. `SQRT_2`'s f32 is a DIFFERENT bit
/// pattern, so the lints (which suggest it / its lower precision) are deliberately suppressed.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub const FIELD_LIPSCHITZ_L: f32 = 1.41421356;

/// The miss distance bound (`if (t > T_MAX) { ... break; }`). Mirrors the GPU's `T_MAX`
/// (`sdf_gbuffer_composite.hlsl:408`). Spelled SYMBOLICALLY (`T_MAX`, NOT `10.0`).
pub const T_MAX: f32 = 10.0;

/// Runs ONE B1 over-relaxation marcher STEP — the Keinert over-relaxed step with the
/// Lipschitz-aware SOR-FAIL retreat-to-plain, plus the `t > T_MAX` miss test — mutating the carried
/// marcher state (`t`/`omega`/`safe_t`/`sor_prev`/`sor_step_prev`/`exhausted`) in place. Authored
/// ONCE over the control-flow axis `C`. Mirrors the GPU B1 marcher's L1459-1498 step span
/// statement-for-statement (the enclosing `for (uint it...)` header + the mesh-guard + the brick
/// islands + the `d < EPS` accept stay hand-written inline; the BUG-B1-HOLE rationale travels in
/// this module's doc).
///
/// `d` is THIS iteration's already-sampled field value (`float d = sdf(p);` — a hand-written
/// statement above the span, passed in as a parameter; NO field call inside the span). `it` is the
/// hand-written loop's CAPTURED induction variable (a suppressed-decl `uint` read as a bare `it`).
/// `t`/`omega`/`safe_t`/`sor_prev`/`sor_step_prev` are the carried `float` vars the hand-written
/// preamble declares ([`Cf::decl_param`] suppressed-decl, so get/set spell `t`/`t = ...;` but NO
/// `float t = ...;` redecl is recorded inside the span); `exhausted` the carried `bool`
/// ([`Cf::decl_bool_param`] suppressed-decl).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; Continue }`. The two loop-control
/// tokens are the mid-body `continue` (the sor-retreat) and the `t > T_MAX` `break` (the miss),
/// propagated through [`Cf::if_`]'s `?`. On Eval the IIFE's `?` short-circuits the live tail exactly
/// like the GPU `continue`/`break` (the carried vars hold the iteration's final values); on Emit
/// every branch is recorded structurally (`?` never early-returns), so the whole span is captured.
/// The returned [`Flow`] is the iteration's disposition (`Break(Continue)` = retreat,
/// `Break(Break)` = miss, `Continue` = fell through the step) — observed by the Eval oracle; on Emit
/// it is `Continue` (discarded by the producer, which records the span by name).
//
// The 8 params are INTRINSIC: `d` + `it` are the iteration inputs, and the 6 carried-state handles
// (`t`/`omega`/`safe_t`/`sor_prev`/`sor_step_prev`/`exhausted`) are the EXACT B1 marcher state the
// committed span reads/writes — each a separate `&C::Var`/`&C::BoolVar` (the established per-var
// suppressed-decl discipline the prior B1 rungs use). Bundling them into a struct would diverge the
// body from that discipline (and from the committed span's flat state) for no benefit, so the
// too_many_arguments lint is suppressed here.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn b1_sor_retreat_body<C: Cf>(
    d: C::Scalar,
    it: C::Uint,
    t: &C::Var,
    omega: &C::Var,
    safe_t: &C::Var,
    sor_prev: &C::Var,
    sor_step_prev: &C::Var,
    exhausted: &C::BoolVar,
) -> Flow {
    let run = || -> Flow {
        // if (omega > 1.0) { <over-relax> } else { t = t + d; }  — the 0%-gate: at `omega == 1.0`
        // the else-arm is the VERBATIM frozen plain step.
        C::if_else(
            C::get_var(omega).gt(C::Scalar::lit(1.0)),
            || -> Flow {
                // float step_len = d * omega;  — a NAMED `float` temp (the committed materializes
                // the over-relaxed step length as a local before the retreat test + the step).
                let step_len = C::temp_float("step_len", d.mul(C::get_var(omega)));
                // if (it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev) { ... continue; }
                // — the sor-fail retreat. `it > 0u` is the LOAD-BEARING budget guard; the `<` is the
                // Lipschitz-aware overlap test (`FIELD_LIPSCHITZ_L * sor_step_prev` is an INLINE
                // `Mul`, NOT a temp — the FMA pin). The composite then-block records the plain-resume
                // `t = safe_t + sor_prev;`, the permanent `omega = 1.0;`, and the `continue;` IN
                // ORDER (the `?`-propagated `cont` skips the live tail on Eval).
                C::if_(
                    C::and2(
                        C::ugt(it, C::uint_lit(0)),
                        C::get_var(sor_prev).add(d).lt(C::named_lit("FIELD_LIPSCHITZ_L", FIELD_LIPSCHITZ_L)
                            .mul(C::get_var(sor_step_prev))),
                    ),
                    || {
                        // t = safe_t + sor_prev;  — plain-resume one certified step past the safe probe.
                        C::set_var(t, C::get_var(safe_t).add(C::get_var(sor_prev)));
                        // omega = 1.0;  — permanent fall-to-plain for the rest of this ray.
                        C::set_var(omega, C::Scalar::lit(1.0));
                        C::cont()
                    },
                )?;
                // safe_t = t;  — remember THIS probe point (the OLD `t`, BEFORE the step below).
                C::set_var(safe_t, C::get_var(t));
                // sor_prev = d;  — the exact field value sampled at safe_t.
                C::set_var(sor_prev, d);
                // sor_step_prev = step_len;  — the over-relaxed step length just taken.
                C::set_var(sor_step_prev, step_len);
                // t = t + step_len;  (R1: byte-identical to the committed `t += step_len`).
                C::set_var(t, C::get_var(t).add(step_len));
                Flow::Continue(())
            },
            || -> Flow {
                // t = t + d;  — the frozen plain arm (R1: byte-identical to the committed `t += d`).
                C::set_var(t, C::get_var(t).add(d));
                Flow::Continue(())
            },
        )?;
        // if (t > T_MAX) { exhausted = false; break; }  — the clear-miss termination (NOT budget
        // exhaustion). The composite then-block records the bare bool assign THEN the break, in order.
        C::if_(C::get_var(t).gt(C::named_lit("T_MAX", T_MAX)), || {
            C::set_bool_var(exhausted, false);
            C::brk()
        })?;
        Flow::Continue(())
    };
    // Return the iteration's disposition: on Eval the `?` short-circuited the IIFE on a
    // continue/break (so the carried vars hold the iteration's final state); on Emit every branch
    // was recorded structurally and this is `Continue` (discarded by the producer).
    run()
}
