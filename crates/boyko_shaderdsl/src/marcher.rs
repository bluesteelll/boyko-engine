//! The B1 main-marcher loop's remaining straight-line GLUE spans (Track B Increment 4g-g1:
//! literal completeness — single-sourcing the last hand-written fragments of the production
//! `for (uint it...)` loop's STRAIGHT-LINE seam that are not a brick island or the accept wrapper).
//!
//! The production B1 over-relaxation marcher (`sdf_gbuffer_composite.hlsl`'s main `for (uint
//! it...)` loop) is, after the prior B1 rungs (the two preamble `bool` decls — Inc 4d; the inner
//! accept-refine `[loop]` — Inc 4c; the SOR-fail-retreat STEP — Inc 4f; the exhaustion re-march —
//! Inc 4e), single-sourced EXCEPT for three irreducible hand-written regions and two tiny
//! STRAIGHT-LINE glue fragments. The irreducible regions stay hand-written:
//!   - the M1 empty-skip island (`if (pc.brick_enabled != 0u) { ... }`) — a resource-binding ladder
//!     (the N separate `PointerGrid`/`PointerGrid1`/`PointerGrid2` resources HLSL cannot dynamically
//!     index);
//!   - the M2 trilinear-surface island (`if (pc.brick_trilinear != 0u) { ... }`) — likewise;
//!   - the `if (d < EPS) { hit = true; exhausted = false; <accept-refine span>; break; }` accept
//!     WRAPPER — it NESTS the already-generated Inc-4c accept-refine span; absorbing it is the
//!     deferred g2 restructure, out of scope here.
//!
//! This module authors the TWO remaining glue fragments ONCE over the control-flow axis `C: Cf`:
//!   - SPAN A ([`b1_marcher_mesh_p_body`]) — the mesh-occlusion guard + the probe-point compute,
//!     contiguous right after the loop header `{` and BEFORE the M1 island:
//!     ```hlsl
//!     if (t >= t_mesh) {
//!         exhausted = false;
//!         break;
//!     }
//!     float3 p = ro + rd * t;
//!     ```
//!   - SPAN B ([`b1_marcher_fold_d_body`]) — the analytic fold's distance sample, AFTER the M2
//!     island and BEFORE the `if (d < EPS)` accept wrapper:
//!     ```hlsl
//!     float d = sdf(p);
//!     ```
//!
//! NO new control-flow / value machinery: every facet is a PROVEN reuse. SPAN A reuses the FLOAT
//! mesh guard `t >= t_mesh` ([`crate::scalar::FieldScalar::ge`]), the in-guard `exhausted = false;`
//! ([`Cf::set_bool_var`] over a SUPPRESSED-DECL bool — [`Cf::decl_bool_param`], the
//! [`crate::decl::b1_decl_exhausted_body`] name), the `brk` ([`Cf::brk`]), and the NAMED `float3 p`
//! temp ([`Cf::temp_vec3`] over `ro + rd * t` — the SAME shape the Inc-4e re-march builds `p` with).
//! `p` is READ-ONLY inside the loop (declared once at the loop top, never reassigned), so it is a
//! `temp_vec3` (a `float3 p = ...;` DeclTemp), NOT a mutable `decl_var_vec3`. The hand-written M1/M2
//! islands read `p` by NAME — the established cross-splice name-sharing (like the generated bool
//! decls the hand-written reads consume). SPAN B reuses the field-call seam ([`Cf::call1`], interned
//! `"sdf"` — the ANALYTIC field, NOT `field_distance`) into a NAMED `float d` temp
//! ([`Cf::temp_float`]); the hand-written `if (d < EPS)` accept wrapper reads `d` by name.
//!
//! Each span is a separate emit with FRESH recorder state, so the cross-splice names (`p` declared
//! by SPAN A, read by SPAN B and the islands; `d` declared by SPAN B, read by the accept wrapper)
//! are seeded BY NAME in each producer: SPAN A declares `p`; SPAN B re-seeds `p` as a CAPTURED
//! `float3` input.
//!
//! Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — the CPU oracle (real `if`/`break`/`Cell` + the host `sdf` closure threaded as
//!     `field` for SPAN B, so [`Cf::call1`]'s `unreachable!` is never reached). SPAN A's mesh-guard
//!     control flow (the `t >= t_mesh` break setting `exhausted = false;`) is reproduced; SPAN B's
//!     `d = sdf(p)` threads the host field.
//!   - `<EmitCf>` — the HLSL recorder; the printers
//!     (`crate::emit::emit_hlsl_b1_marcher_mesh_p` / `crate::emit::emit_hlsl_b1_marcher_fold_d`)
//!     walk the STMT IR into the spliced spans (byte-identical to the committed `.comp.spv`, proven
//!     by the cmp-`.spv`).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// Authors SPAN A — the B1 marcher's mesh-occlusion guard + the probe-point compute (the committed
/// `sdf_gbuffer_composite.hlsl` fragment right after the `for (uint it...)` loop header `{` and
/// BEFORE the M1 brick island) ONCE over the control-flow axis `C`, mutating the carried `exhausted`
/// flag in place and returning the NAMED `float3 p` probe point (so the Eval oracle observes it).
///
/// `ro` / `rd` are the world ray origin/direction; `t` the marcher's carried world distance (a
/// SUPPRESSED-DECL [`Cf::decl_param`], declared by the hand-written B1 preamble); `t_mesh` the
/// mesh-occlusion depth bound (a CAPTURED `float`); `exhausted` the BUG-B1-HOLE-3 budget-exhaustion
/// flag (a SUPPRESSED-DECL [`Cf::decl_bool_param`], the [`crate::decl::b1_decl_exhausted_body`]
/// name). The body is two recorded statements: the mesh-guard `if (t >= t_mesh) { exhausted = false;
/// break; }` (a COMPOSITE then-block recording the `exhausted = false;` assign THEN the `break;` in
/// order — the SAME `set-before-brk` shape the Inc-4e re-march accept uses) and the `float3 p = ro +
/// rd * t;` NAMED temp ([`Cf::temp_vec3`]). The mesh-guard's `?` propagates the `brk` token, which
/// the enclosing hand-written `for (uint it...)` loop consumes; here the span is FUNCTION-SCOPE (the
/// IIFE `run`), so the `?` only short-circuits the IIFE — `p` is computed AFTER the guard, which on
/// Eval means the IIFE returns early on a mesh-occluded ray (no `p` computed) and the producer reads
/// the carried result instead. On Emit every statement is recorded structurally (`?` never early-
/// returns), so the whole span — guard THEN `p` decl — is captured in program order.
///
/// `p` is READ-ONLY in the production loop (never reassigned), so it materializes as a `float3 p =
/// ...;` DeclTemp ([`Cf::temp_vec3`]), NOT a mutable `decl_var_vec3` — the hand-written M1/M2 islands
/// + SPAN B read it by NAME (the established cross-splice name-sharing).
#[inline]
pub fn b1_marcher_mesh_p_body<C: Cf>(
    ro: C::Vec3f,
    rd: C::Vec3f,
    t: &C::Var,
    t_mesh: C::Scalar,
    exhausted: &C::BoolVar,
) -> C::Vec3f {
    // if (t >= t_mesh) { exhausted = false; break; }  — the mesh-occlusion guard. The composite
    // then-block records BOTH statements IN ORDER (the `exhausted = false;` assign THEN the
    // `break;`): on Eval `set_bool_var` writes `exhausted` BEFORE the `brk` token is `?`-propagated;
    // on Emit the then-block is EXACTLY the two committed statements. The committed text spells `t >=
    // t_mesh` (t LEFT), so `ge` emits `Ge(get_var(t), t_mesh)`. The `?` short-circuits the IIFE; the
    // enclosing hand-written `for` loop consumes the break.
    let run = || -> Flow {
        C::if_(C::get_var(t).ge(t_mesh), || {
            C::set_bool_var(exhausted, false);
            C::brk()
        })?;
        Flow::Continue(())
    };
    let _ = run();

    // float3 p = ro + rd * t;  — the NAMED `float3` probe point (the committed materializes it as a
    // named local; READ-ONLY in the loop, so a `temp_vec3`, not a mutable var). The hand-written
    // M1/M2 islands + SPAN B read `p` by name.
    C::temp_vec3("p", C::vec3_add(ro, C::vec3_mul_scalar(rd, C::get_var(t))))
}

/// Authors SPAN B — the B1 marcher's analytic fold distance sample (the committed
/// `sdf_gbuffer_composite.hlsl` `float d = sdf(p);` AFTER the M2 brick island and BEFORE the
/// `if (d < EPS)` accept wrapper) ONCE over the control-flow axis `C` plus the field-call seam
/// `field`, returning the NAMED `float d` (so the Eval oracle observes it; the hand-written accept
/// wrapper reads `d` by name on the splice).
///
/// `p` is the probe point — a CAPTURED `float3` (SPAN A declared `float3 p = ...;` above; each span
/// is a separate emit with FRESH recorder state, so `p` is re-seeded by NAME here as a `float3`
/// input). `field` is the field-distance seam: on Eval it is the host `sdf` closure (so
/// `b1_marcher_fold_d_body::<EvalCf>` runs the host field at `p`); on Emit it records a `sdf(p)` call
/// node (via [`Cf::call1`] with the interned `"sdf"` callee — the ANALYTIC field, NOT
/// `field_distance`). The body is ONE recorded statement: the `float d = sdf(p);` NAMED temp
/// ([`Cf::temp_float`]).
#[inline]
pub fn b1_marcher_fold_d_body<C: Cf, F: Fn(C::Vec3f) -> C::Scalar>(p: C::Vec3f, field: F) -> C::Scalar {
    // float d = sdf(p);  — a NAMED `float` temp via the threaded `field` seam (the analytic field via
    // the hand-written `sdf`, interned `"sdf"`). The hand-written `if (d < EPS)` accept wrapper reads
    // `d` by name.
    C::temp_float("d", field(p))
}
