//! The SDFDDGI I2 probe-update sphere-trace leaf — a STRIPPED fixed-budget field march.
//!
//! `probe_march` is the GI-ray marcher the I2 probe-update pass runs once per Fibonacci ray:
//! a fixed-budget (`GI_MAX_IT`) sphere-trace over the FROZEN `field_distance` call-seam that
//! returns only `(hit, t)` — whether the ray struck an occluder and the hit distance (or the
//! escape bound on a sky miss). It is the shadow marcher's `[loop]`
//! ([`crate::shadow::sdf_soft_shadow_body`]) STRIPPED of everything a GI ray does not need:
//! NO penumbra min-tracking, NO brick/mesh-guard, NO gbuffer-store, NO accept-refine, NO
//! `sdf_soft_shadow_ranged` call. A GI ray wants the true nearest-surface march (full step
//! `d`, not a cone step) to a coarse hit + distance; surface-quality refinement is a
//! primary-visibility concern the resolve marcher owns, not this pass (plan §1.2).
//!
//! # The frozen-span firewall
//!
//! The body consumes `field_distance` ONLY through the existing [`Cf::call1`] seam (identical
//! to [`crate::shadow`] / [`crate::normal`]) — it does NOT touch the frozen
//! `sdf_gbuffer_composite.hlsl` marcher spans, `sdf_soft_shadow`, or `oct_encode`. Each
//! `field_distance` call is itself a `[loop]` over `n = min(Buf[0], MAX_SDF_EDITS = 16)`
//! edits (the marcher's own CSG fold): the outer march is 2 predictable compares per step, but
//! the REAL hot loop is that 16-deep, SSBO-fetch-bound inner edit fold — the dominant cost
//! term the I2 bench sizes against (plan §1.2).
//!
//! # The `(hit, t)` out-deposit shape
//!
//! `probe_march` returns TWO results — `hit` (occluder struck?) and `t` (the hit / escape
//! distance) — via the proven `m2_surface_hit` deposit shape ([`crate::surface`]): `hit` is
//! the `bool` function return ([`Cf::RetCellB`] / [`Cf::ret_b`]) and `t` is an `out float`
//! ([`Cf::OutFloat`] / [`Cf::out_float_assign`]), folded by the composite
//! [`Cf::if_hit_ret_b`] at the occluder-hit site (`if (d < GI_HIT_EPS) { t = <t>; return
//! true; }`). The sky-escape / budget-exhaust tail writes `t` then returns `false`.
//!
//! # The named tuning consts spell as SYMBOLS
//!
//! `GI_MINT` / `GI_HIT_EPS` / `GI_MINT_STEP` / `GI_T_MAX` spell SYMBOLICALLY in the emitted
//! HLSL (via [`Cf::named_lit`]), and `GI_MAX_IT` is the `[loop]` bound symbol — the
//! bench-tunable knob re-DXC'd per sweep value {32, 64, 96, 128} (plan §1.2, the SSAO-variant
//! mechanism → measured==shipped). A value-spelled const would change the committed
//! `OpConstant` set. The Eval values below drive ONLY the CPU oracle.
//!
//! # `R1` (no `+=` leaf)
//!
//! The step accumulation is `t = t + max(d, GI_MINT_STEP)` (the eDSL's natural [`Cf::set_var`]
//! form), the proven-byte-identical R1 add-form the shadow marcher's `t += max(...)` uses — so
//! NO compound-assign leaf is added.
//!
//! Instantiated two ways (the established control-axis discipline):
//!   - `<EvalCf>` — the CPU oracle (real `for`/`if`/`break`/`Cell` + the host `field_distance`
//!     closure threaded as `field`), the unit-test fixture proving unit-sphere hit / sky escape.
//!   - `<EmitCf>` — the HLSL recorder ([`crate::emit::emit_hlsl_probe_march`]) walking the STMT
//!     IR into the `[loop]`+tail span spliced into `sdf_probe_update.comp.hlsl`.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The march start offset (`float t = GI_MINT;`) — the SHADOW_MINT-class start bias keeping
/// the first sample off the origin surface. Spelled SYMBOLICALLY (`GI_MINT`). The Eval value
/// mirrors the committed shader literal; it drives ONLY the Eval march oracle.
pub const GI_MINT: f32 = 16.0 * 0.0005;

/// The occluder-hit threshold (`if (d < GI_HIT_EPS) { t = t; return true; }`). A ray whose
/// field sample drops below this has struck a surface. Spelled SYMBOLICALLY (`GI_HIT_EPS`).
pub const GI_HIT_EPS: f32 = 2.0 * 0.001;

/// The minimum per-step advance — the floor on the full field step `d` (`t = t + max(d,
/// GI_MINT_STEP)`), preventing a stall in a shallow-gradient region. Spelled SYMBOLICALLY
/// (`GI_MINT_STEP`).
pub const GI_MINT_STEP: f32 = 16.0 * 0.0005;

/// The escape / miss-distance bound (`if (t > GI_T_MAX) { break; }`) — beyond it the ray has
/// escaped to sky (`hit == false`). Spelled SYMBOLICALLY (`GI_T_MAX`). The GI reach is the
/// bounded probe volume, so this mirrors the marcher's `T_MAX`.
pub const GI_T_MAX: f32 = 10.0;

/// The max march steps per GI ray — `probe_march`'s `[loop]` trip count, the BOUND SYMBOL the
/// for-header carries. Spelled SYMBOLICALLY in the emitted HLSL header (`GI_MAX_IT`, NOT
/// `64u`). This is the bench-tunable knob: the emitter bin re-DXCs one variant per sweep value
/// {32, 64, 96, 128} so the measured shader IS the shipped shader (plan §1.2 / §5). The Eval
/// value is the sweep midpoint — it drives ONLY the Eval oracle's trip count.
pub const GI_MAX_IT: usize = 64;

/// Sphere-traces a GI ray from `ro` along `rd` over the FROZEN `field_distance` seam,
/// depositing the marched hit distance `t` into the `out float` `t_out` and returning the
/// occluder-hit flag through `hit_out`. Authored ONCE over the control-flow axis `C` plus the
/// field-call seam `field`. A STRIPPED clone of [`crate::shadow::sdf_soft_shadow_body`]'s
/// loop+tail (plan §1.2): the penumbra min-track, the cone-step divisor, and the `clamp01`
/// visibility tail are REMOVED — a GI ray returns only `(hit, t)`.
///
/// `ro` is the ray origin (the probe world position), `rd` the (normalized) ray direction.
/// `field` is the field-distance seam (see [`crate::shadow`]): on Eval it is the host
/// `field_distance` closure (so `probe_march_body::<EvalCf>` re-runs the host field at each
/// `ro + rd * t`); on Emit it records a `field_distance(ro + rd * t)` call node (via
/// [`Cf::call1`]). `hit_out` receives `true` on an occluder hit (`d < GI_HIT_EPS`), else
/// `false` (the ray escaped past `GI_T_MAX` or exhausted `GI_MAX_IT`); `t_out` receives the
/// distance at which the loop stopped.
///
/// The body is a FUNCTION-SCOPE IIFE so the in-loop occluder-hit early-return (the composite
/// [`Cf::if_hit_ret_b`] writing `t_out` then `return true;`) forwards through
/// [`Cf::runtime_for`]'s `?` to the IIFE's `?`, skipping the tail. The `t > GI_T_MAX` break is
/// a [`Cf::brk`] the loop CONSUMES, so the post-loop `t_out` write + `return false;`
/// (the sky-escape / budget-exhaust case) RUNS. On Emit every branch is recorded structurally.
#[inline]
pub fn probe_march_body<C: Cf, F: Fn(C::Vec3f) -> C::Scalar>(
    ro: C::Vec3f,
    rd: C::Vec3f,
    field: F,
    t_out: &C::OutFloat,
    hit_out: &C::RetCellB,
) {
    let run = || -> Flow {
        // float t = GI_MINT;  — the march start bias (a TRUE local, a recorded DeclVar).
        let t = C::decl_var("t", C::named_lit("GI_MINT", GI_MINT));

        C::runtime_for("[loop]", "i", "GI_MAX_IT", GI_MAX_IT, |_i| -> Flow {
            // float d = field_distance(ro + rd * t);
            let d = C::temp_float(
                "d",
                field(C::vec3_add(ro, C::vec3_mul_scalar(rd, C::get_var(&t)))),
            );
            // if (d < GI_HIT_EPS) { t_out = t; return true; }  — the occluder-hit deposit (the
            // composite `m2_surface_hit` shape: the out-float write THEN the bool return, in
            // order). Forwarded through runtime_for to the IIFE by `?`.
            let hit_eps = C::named_lit("GI_HIT_EPS", GI_HIT_EPS);
            C::if_hit_ret_b(t_out, hit_out, d.lt(hit_eps), C::get_var(&t))?;
            // t = t + max(d, GI_MINT_STEP);  — the full-step advance (R1 add-form; GI rays want
            // the true nearest-surface step, not the shadow marcher's `d / FIELD_LIPSCHITZ_L`
            // cone divisor).
            let step = C::named_lit("GI_MINT_STEP", GI_MINT_STEP);
            C::set_var(&t, C::get_var(&t).add(d.max(step)));
            // if (t > GI_T_MAX) { break; }  — the sky escape; runtime_for CONSUMES the `brk`, so
            // the post-loop deposit (return false) runs.
            let tmax = C::named_lit("GI_T_MAX", GI_T_MAX);
            C::if_(C::get_var(&t).gt(tmax), C::brk)?;
            Flow::Continue(())
        })?;
        // The sky-escape / budget-exhaust tail: deposit the stopped `t`, then `return false;`
        // (no occluder was hit within the budget).
        C::out_float_assign(t_out, C::get_var(&t));
        C::ret_b(hit_out, false)?;
        Flow::Continue(())
    };
    // On Eval the early `?` (occluder hit) already wrote `t_out` + `hit_out`; the tail runs
    // only on escape/exhaust. On Emit the recorder captured every statement.
    let _ = run();
}
