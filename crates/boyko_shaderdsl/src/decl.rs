//! The B1 marcher's `bool` PREAMBLE-DECL leaves (Increment 4d: the FIRST rung of the
//! B1-marcher single-source ladder).
//!
//! The production B1 over-relaxation marcher (`sdf_gbuffer_composite.hlsl`'s main `for (uint
//! it...)`) carries two `bool` preamble flags declared above the loop:
//!   - `bool hit = false;`        (L1316) — set true on a converged accept;
//!   - `bool exhausted = true;`   (L1327) — the BUG-B1-HOLE-3 budget-exhaustion flag.
//!
//! They are NON-CONTIGUOUS in the committed shader: L1316 and L1327 are separated by 4 `float`
//! decls (`safe_t`/`sor_prev`/`sor_step_prev`) + the 7-line BUG-B1-HOLE-3 rationale comment, so a
//! single sentinel pair is geometrically impossible — each `bool` decl is its OWN single-sourced
//! span (the `float` decls + ALL comments between them stay hand-written). This module authors the
//! two decls ONCE over the control-flow axis `C: Cf` via the TYPED [`Cf::decl_bool_var`] facet
//! (Inc 4d — `decl_var` hardcodes `float`); the printers
//! (`crate::emit::emit_hlsl_b1_decl_hit` / `crate::emit::emit_hlsl_b1_decl_exhausted`) walk
//! each one-statement body into its `bool <name> = <init>;` line.
//!
//! Each body is a ONE-STATEMENT decl (no loop, no return, no field call) — the smallest possible
//! eDSL leaf. Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — returns the `Cell<bool>` handle holding the init value (the decl round-trip
//!     the 4d eval test asserts; the real proof is the cmp-`.spv`);
//!   - `<EmitCf>` — records the `Stmt::DeclVar { ty: Bool, .. }` the printer spells.
//!
//! The reads of these flags (`if (hit ...)` L1557, `if (exhausted)` L1513) and the in-loop sets
//! (`hit = true;`, `exhausted = false;`) are in HAND-WRITTEN / out-of-scope code — the single-
//! sourced spans here only DECLARE the flags, so no `get_bool_var` / `set_bool_var` facet is
//! needed at this rung (the in-loop `hit = true;` consumer lands in Inc 4e).

use crate::cf::Cf;

/// Authors the B1 marcher's `bool hit = false;` preamble decl (the committed
/// `sdf_gbuffer_composite.hlsl:1316`) ONCE over the control-flow axis `C`, returning the declared
/// flag's [`Cf::BoolVar`] handle. A one-statement body: a single [`Cf::decl_bool_var`] init to
/// `false`. On Eval the handle is a `Cell<bool>` holding `false` (the decl round-trip); on Emit it
/// records the `bool hit = false;` `Stmt::DeclVar` (the printer spells the line).
#[inline]
pub fn b1_decl_hit_body<C: Cf>() -> C::BoolVar {
    // bool hit = false;  — the converged-accept flag (set true by the hand-written in-loop accept,
    // out of this single-sourced span).
    C::decl_bool_var("hit", false)
}

/// Authors the B1 marcher's `bool exhausted = true;` preamble decl (the committed
/// `sdf_gbuffer_composite.hlsl:1327`) ONCE over the control-flow axis `C`, returning the declared
/// flag's [`Cf::BoolVar`] handle. A one-statement body: a single [`Cf::decl_bool_var`] init to
/// `true`.
///
/// `exhausted` is the BUG-B1-HOLE-3 budget-exhaustion flag: it starts `true` and is cleared by
/// EVERY in-loop `break` (mesh-occlusion, convergence, `t > T_MAX`), so it remains `true` exactly
/// when the marcher `for` falls off the end without ANY break — the precise, minimal re-march
/// trigger. The init MUST be `true` (under-detecting exhaustion reopens the hole). The clears live
/// in the hand-written loop body, out of this single-sourced decl span.
#[inline]
pub fn b1_decl_exhausted_body<C: Cf>() -> C::BoolVar {
    // bool exhausted = true;  — init true, cleared by every in-loop break (BUG-B1-HOLE-3).
    C::decl_bool_var("exhausted", true)
}
