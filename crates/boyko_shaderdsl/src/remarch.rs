//! The B1 EXHAUSTION RE-MARCH inner-loop leaf (Increment 4e: the production main-marcher's
//! BUG-B1-HOLE-3 re-march `[loop]` SPAN re-expressed in the control eDSL).
//!
//! # BUG-B1-HOLE-3 rationale (the WHY travels with the source-of-truth)
//!
//! The B1 over-relaxation marcher (`sdf_gbuffer_composite.hlsl`'s main `for (uint it...)`)
//! sphere-traces with an over-relaxation factor `omega > 1`, which can take MORE steps than the
//! frozen `omega == 1` plain marcher (the `steps(omega) <= steps(1)` bound is genuinely violated
//! and unbounded). So the over-relaxed pass can EXHAUST the `MAX_IT` budget mid-field on a ray the
//! frozen plain marcher would have HIT. When that happens — `exhausted` is still `true`: the
//! marcher `for` fell off the end with NO break (it neither converged nor left the scene nor hit
//! the mesh) — this span RE-MARCHES from the ORIGINAL seed with a plain `omega == 1.0` sphere-trace
//! (`t += d`, the EXACT frozen marcher body, no omega / no sor logic) and uses ITS result. Hence B1
//! reports "no hit" only where BOTH passes miss — exactly where the frozen marcher misses: B1's
//! hit-set is identical to the frozen hit-set with NO dependence on any step-count bound. At
//! `omega == 1.0` the fast pass IS the frozen plain loop, so on exhaustion this re-march reproduces
//! the identical frozen `(hit = false)` result — the `omega == 1.0` output is byte-unchanged (the
//! 0%-gate). Over-detecting `exhausted` is harmless (a clear-miss re-march just misses again);
//! under-detecting would reopen the hole.
//!
//! This module authors the INNER RE-MARCH `[loop]` SPAN ONCE over the control-flow axis `C: Cf`
//! plus the field-call seam `field` — the committed `[loop] for (uint it2...) { ... t += d; }` of
//! L1520-1535. The enclosing `if (exhausted) { t = t_seed; hit = false; ... }` wrapper (the
//! re-seed, the `hit = false;` reset, the BUG-B1-HOLE-3 rationale comment, and the `if (exhausted)`
//! brace) stays HAND-WRITTEN inline in the committed shader (framing (b) — the generated text is
//! spliced between the `// === GENERATED b1_exhaustion_remarch BEGIN/END ===` sentinels, around
//! which the wrapper lives un-generated).
//!
//! It is a near-CLONE of the proven [`crate::refine::b1_accept_refine_body`] (Inc 4c), with 4 small
//! new facets: the FLOAT mesh guard `t >= t_mesh` ([`crate::FieldScalar::ge`]); a NAMED `float3 p`
//! temp ([`Cf::temp_vec3`], vs the inline `ro + rd * t` of `b1_accept_refine`); the in-loop
//! `hit = true;` ([`Cf::set_bool_var`]) carried by a SUPPRESSED-DECL bool ([`Cf::decl_bool_param`],
//! whose value is read back by [`Cf::get_bool_var`] for the Eval oracle's `(hit, t)` tuple). The
//! field seam interns `"sdf"` (the ANALYTIC field via the hand-written `sdf`, NOT `field_distance`),
//! and there is NO return facet — the only loop control is the three [`Cf::brk`]s (the mesh guard,
//! the `d < EPS` accept, and the `t > T_MAX` miss); the marcher CONTINUES past the inner loop.
//!
//! Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — the CPU oracle (real `for`/`if`/`break`/`Cell` + the host `sdf` closure threaded
//!     as `field`, so [`Cf::call1`]'s `unreachable!` is never reached). The eval sweep reproduces
//!     the committed re-march CONTROL FLOW (scoped to control flow — the cmp-`.spv` is the
//!     byte-identity oracle).
//!   - `<EmitCf>` — the HLSL recorder; the printer
//!     ([`crate::emit::emit_hlsl_b1_exhaustion_remarch`]) walks the STMT IR into the `[loop]` span
//!     (byte-identical to the committed `.comp.spv`, proven by the cmp-`.spv`).
//!
//! # The named tuning consts spell as SYMBOLS
//!
//! `EPS` / `T_MAX` spell SYMBOLICALLY in the emitted HLSL (via [`Cf::named_lit`]), and `MAX_IT` is
//! the `[loop]` bound symbol — a value-spelled const would change the committed `OpConstant` set.
//! The Eval values below mirror the committed shader literals
//! (`sdf_gbuffer_composite.hlsl:409,407,408`); they drive ONLY the Eval control-flow oracle (the
//! `.spv` gate is unaffected by the Eval value).
//!
//! # `R1` (no `+=` leaf)
//!
//! The step accumulation is `t = t + d` (the eDSL's natural [`Cf::set_var`] form), which the
//! GO/NO-GO spike proved compiles BYTE-IDENTICAL at this exact depth-2 site to the committed
//! `t += d` — so NO compound-assign leaf is added (the SAME R1 result the `b1_accept_refine` and
//! `sdf_soft_shadow` leaves proved).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The plain-march budget — the re-march `[loop]` trip count, the BOUND SYMBOL the for-header
/// carries. Mirrors the GPU's `MAX_IT` (`sdf_gbuffer_composite.hlsl:409`). Spelled SYMBOLICALLY
/// in the emitted HLSL header (`MAX_IT`, NOT `128u`). The Eval `bound_val` MUST equal it for a
/// faithful budget-exhaust case.
pub const MAX_IT: usize = 128;

/// The on-surface accept threshold on `d` (`if (d < EPS) { hit = true; break; }`). Mirrors the
/// GPU's `EPS` (`sdf_gbuffer_composite.hlsl:407`). Spelled SYMBOLICALLY (`EPS`, NOT `0.001`).
pub const EPS: f32 = 0.001;

/// The miss distance bound (`if (t > T_MAX) { break; }`). Mirrors the GPU's `T_MAX`
/// (`sdf_gbuffer_composite.hlsl:408`). Spelled SYMBOLICALLY (`T_MAX`, NOT `10.0`).
pub const T_MAX: f32 = 10.0;

/// RE-MARCHES the B1 marcher from the ORIGINAL seed with a plain `omega == 1.0` sphere-trace (the
/// BUG-B1-HOLE-3 budget-exhaustion recovery), mutating the carried `t`/`hit` in place and returning
/// their final values. Authored ONCE over the control-flow axis `C` plus the field-call seam
/// `field`. Mirrors the GPU B1 marcher's L1520-1535 re-march `[loop]` span statement-for-statement
/// (the enclosing `if (exhausted) { t = t_seed; hit = false; ... }` wrapper + the BUG-B1-HOLE-3
/// rationale comment stay hand-written inline).
///
/// `ro` / `rd` are the world ray origin/direction; `t_seed` the original world `t` the fast pass
/// used (the re-seed value the hand-written preamble assigns to `t`); `t_mesh` the mesh-occlusion
/// depth the loop guards against. `t` and `hit` are the carried vars the hand-written preamble
/// declares (suppressed-decl: [`Cf::decl_param`] for `t`, [`Cf::decl_bool_param`] for `hit`), so
/// get/set spell `t`/`hit`/`t = ...;`/`hit = true;` but NO `float t = ...;`/`bool hit = ...;`
/// redecl is recorded inside the span. `field` is the field-distance seam: on Eval it is the host
/// `sdf` closure (so `b1_exhaustion_remarch_body::<EvalCf>` re-runs the host field at each
/// `ro + rd * t`); on Emit it records a `sdf(p)` call node (via [`Cf::call1`] with the interned
/// `"sdf"` callee).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; Continue }`. There is NO function
/// return facet: the only loop control is the three `brk`s (the `t >= t_mesh` mesh guard, the
/// `d < EPS` accept, and the `t > T_MAX` miss), propagated through [`Cf::if_`]'s `?`;
/// [`Cf::runtime_for`] CONSUMES each (the loop returns `Continue`), so the IIFE falls through and
/// the carried `t`/`hit` hold the final values. The `d < EPS` accept sets `hit = true;` BEFORE the
/// `brk` (the composite then-block records the assign THEN the break, in order). On Emit every
/// branch is recorded structurally (`?` never early-returns); the whole span is captured. The
/// returned `(get_bool_var(&hit), get_var(&t))` is the observed result on Eval; on Emit the tuple
/// is discarded by the producer (the span mutates `hit`/`t` by name).
#[inline]
pub fn b1_exhaustion_remarch_body<C: Cf, F: Fn(C::Vec3f) -> C::Scalar>(
    ro: C::Vec3f,
    rd: C::Vec3f,
    t_seed: C::Scalar,
    t_mesh: C::Scalar,
    field: F,
) -> (bool, C::Scalar) {
    // float t = ...; / bool hit = ...;  (the carried vars the hand-written re-seed declares — a
    // suppressed-decl param + a suppressed-decl bool, so get/set spell `t`/`hit`/`t = ...;`/`hit =
    // ...;` but NO `float t = ...;` / `bool hit = ...;` redecl is recorded inside the span).
    let t = C::decl_param("t", t_seed);
    let hit = C::decl_bool_param("hit", false);

    let run = || -> Flow {
        C::runtime_for("[loop]", "it2", "MAX_IT", MAX_IT, |_it2| -> Flow {
            // if (t >= t_mesh) { break; }  — the mesh-occlusion guard; `if_`'s `?` propagates the
            // `brk` token, which runtime_for CONSUMES (the loop ends; the IIFE falls through). The
            // committed text spells `t >= t_mesh` (t LEFT), so `ge` emits `Ge(get_var(t), t_mesh)`.
            C::if_(C::get_var(&t).ge(t_mesh), C::brk)?;
            // float3 p = ro + rd * t;  — a NAMED `float3` temp (the committed materializes the
            // probe point as a named local BEFORE the `sdf(p)` call, unlike `b1_accept_refine`'s
            // inline `sdf(ro + rd * t)`).
            let p = C::temp_vec3("p", C::vec3_add(ro, C::vec3_mul_scalar(rd, C::get_var(&t))));
            // float d = sdf(p);  — a NAMED `float` temp via the threaded `field` seam (the analytic
            // field via the hand-written `sdf`, interned `"sdf"`).
            let d = C::temp_float("d", field(p));
            // if (d < EPS) { hit = true; break; }  — the on-surface accept. The composite then-block
            // records BOTH statements IN ORDER (the `hit = true;` assign THEN the `break;`): on Eval
            // `set_bool_var` writes `hit` BEFORE the `brk` token is `?`-propagated, so the oracle
            // reads the FRESH `true`; on Emit the then-block is EXACTLY the two committed statements
            // (the `set_var`-before-`brk` ordering `sdf_soft_shadow` proved captures correctly).
            C::if_(d.lt(C::named_lit("EPS", EPS)), || {
                C::set_bool_var(&hit, true);
                C::brk()
            })?;
            // t = t + d;  (R1: byte-identical to the committed `t += d` at this depth-2 site — the
            // spike proved it; no compound-assign leaf). The plain `omega == 1.0` frozen step.
            C::set_var(&t, C::get_var(&t).add(d));
            // if (t > T_MAX) { break; }  — the miss bound.
            C::if_(C::get_var(&t).gt(C::named_lit("T_MAX", T_MAX)), C::brk)?;
            Flow::Continue(())
        })?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the loop consumed its own breaks; on Emit the recorder captured
    // every statement.
    let _ = run();

    // The final `(hit, t)` — the observed Eval result; on Emit the handles, discarded by the producer.
    (C::get_bool_var(&hit), C::get_var(&t))
}
