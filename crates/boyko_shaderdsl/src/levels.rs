//! The M4 clip-map LEVEL-SELECTOR leaf (Increment 5a: the production `select_level` brick-LOD
//! scan re-expressed in the control eDSL — the FIRST signed-`int`-returning leaf).
//!
//! `select_level` (`sdf_gbuffer_composite.hlsl:1221`) is the brick-atlas clip-map level decider:
//! it scans the nested LOD levels finest-first and returns the index of the first one whose
//! axis-aligned box CONTAINS the world point `p` (`int` `0..brick_levels-1`), or `-1` when `p` is
//! outside every active level (the caller folds the analytic field). This module authors the
//! `[unroll]` scan body ONCE over the control-flow axis `C: Cf`, between the
//! `// === GENERATED select_level BEGIN/END ===` sentinels INSIDE `select_level`; the hand-written
//! signature `int select_level(float3 p) {` + the closing `}` stay un-generated (framing (b)).
//!
//! # The signed-`int` return + the M4Level access-text — the two new facets
//!
//! Unlike the prior return-bearing leaves (the `uint` `brick_cell_class`, the `float`
//! `m2_regula_falsi`, the `bool` `m2_surface_hit`), `select_level` returns a SIGNED `int`:
//! - the early `return (int)L;` ([`Cf::if_ret_i`] + [`Cf::int_from_uint`]) — the `(int)L` cast of
//!   the `uint` loop iv;
//! - the tail `return -1;` ([`Cf::ret_i`] + [`Cf::int_lit_signed`]) — a BARE signed literal (NOT a
//!   `<x>u` unsigned), the binary fact the GATE-0 spike read off the committed `.comp.spv`.
//!
//! The `M4Level` array reads (`m2_levels[L].origin_brick_world.xyz` / `.w` /
//! `dims_atlas_dim.xyz`) are spelled by ACCESS TEXT ([`Cf::level_field_vec3`] /
//! [`Cf::level_field_scalar`]) — the struct LAYOUT is NOT modeled — and the push-constant runtime
//! count `pc.brick_levels` by bare-text ([`Cf::pc_uint`]). On Eval these are served by THREADED
//! CLOSURES (the [`Cf::call1`] field-call seam discipline): the `Cf::level_field_*` / `Cf::pc_uint`
//! hooks are the EMIT recorders (`unreachable!` on Eval), so `select_level_body::<EvalCf>` reads
//! the host level fixture through the closures, and `select_level_body::<EmitCf>` records the
//! access-text nodes through the producer's `EmitCf::level_field_*` closures.
//!
//! # The `[unroll]` loop reuses [`Cf::runtime_for`]
//!
//! The committed header `[unroll] for (uint L = 0u; L < BRICK_LEVELS; ++L)` spells a BOUND SYMBOL
//! (`BRICK_LEVELS`), so it reuses [`Cf::runtime_for`] (which prints `<attr>\nfor (uint <iv> = 0u;
//! <iv> < <bound_sym>; ++<iv>)`) with `attr = "[unroll]"`, `iv = "L"`, `bound_sym = "BRICK_LEVELS"`
//! — NOT the `[unroll]`-with-`<n>u`-literal [`Cf::unroll_for`] (which hardcodes the iv `a`). The
//! `[unroll]` attribute makes DXC unroll it (the trip count is the compile-time `BRICK_LEVELS`);
//! the cmp-`.spv` gates that this header re-DXCs byte-identical.
//!
//! `runtime_for` also FORWARDS the in-loop `Break(Return)` (the `return (int)L;`) through its `?`
//! to the function-scope IIFE — the SAME function-return forwarding `m2_surface_hit`'s converged
//! hit uses — so the early return short-circuits the tail `return -1;`. The `L >= pc.brick_levels`
//! early-out is a [`Cf::brk`] propagated through [`Cf::if_`]'s `?`; `runtime_for` CONSUMES it, so
//! the post-loop tail runs.
//!
//! # Instantiation (the established control-axis discipline)
//!
//! - `<EvalCf>` — the CPU oracle (real `for`/`if`/`break`/`Cell` + the host level fixture threaded
//!   through the closures). The eval sweep reproduces the committed `select_level` CONTROL FLOW —
//!   the per-level containment scan, the boundary exclusion (`p == hi` excluded by `<`), the
//!   `L >= brick_levels` skip — to-bits against a host mirror (an `i32`, so EXACT eq).
//! - `<EmitCf>` — the HLSL recorder; the printer ([`crate::emit::emit_hlsl_select_level`]) walks
//!   the STMT IR into the scan body (byte-identical to the committed `.comp.spv`, proven by the
//!   cmp-`.spv`).
//!
//! # `R1` (no compound-assign)
//!
//! Every value is a fresh `decl_var` (`o` / `bw` / `hi`) — no `+=` form — so there is no R1 concern.

use crate::cf::{Cf, Flow};

/// The number of nested clip-map levels — the `[unroll]` scan's compile-time bound. Mirrors the
/// GPU's `static const uint BRICK_LEVELS = 3u;` (`sdf_gbuffer_composite.hlsl:177`) and the host
/// `boyko_sdf_math::brick::BRICK_LEVELS`. Spelled SYMBOLICALLY in the emitted HLSL header
/// (`BRICK_LEVELS`, NOT `3u`) — a value-spelled bound would change the committed loop header text.
pub const BRICK_LEVELS: usize = 3;

/// Scans the nested clip-map levels finest-first for the tightest LOD whose box CONTAINS `p`,
/// depositing the SIGNED level index (`0..brick_levels-1`) or `-1` (outside every active level)
/// into `ret_out`. Authored ONCE over the control-flow axis `C` plus three fixture seams. Mirrors
/// the GPU `select_level`'s L1222-1234 scan statement-for-statement (the hand-written signature +
/// closing brace stay un-generated).
///
/// `p` is the world query point. `level_vec3` / `level_scalar` read `m2_levels[L].<field>` (a
/// `float3` member+swizzle / a `.w` scalar); `pc_brick_levels` reads `pc.brick_levels`. On Eval
/// they index/read the host level fixture (so `Cf::level_field_*` / `Cf::pc_uint`'s `unreachable!`
/// is never reached); on Emit the producer passes closures that record the access-text nodes (via
/// `EmitCf::level_field_*` / `EmitCf::pc_uint`).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; ret_i(ret_out, -1)?; Continue }`, so
/// an in-loop [`Cf::if_ret_i`]'s `Break(Return)` (the `all(p >= o) && all(p < hi)` containment hit)
/// forwards through [`Cf::runtime_for`]'s `?` to the IIFE's `?` — skipping the tail `return -1;`.
/// The `L >= pc.brick_levels` early-out is a [`Cf::brk`] propagated through [`Cf::if_`]'s `?`;
/// `runtime_for` CONSUMES it, so the post-loop `return -1;` tail RUNS. On Emit every branch is
/// recorded structurally (`?` never early-returns); the whole scan is captured.
#[inline]
pub fn select_level_body<C, FV, FS, FP>(
    p: C::Vec3f,
    level_vec3: FV,
    level_scalar: FS,
    pc_brick_levels: FP,
    ret_out: &C::RetCellI,
) where
    C: Cf,
    FV: Fn(C::Iv, &'static str) -> C::Vec3f,
    FS: Fn(C::Iv, &'static str) -> C::Scalar,
    FP: Fn() -> C::Uint,
{
    let run = || -> Flow {
        C::runtime_for("[unroll]", "L", "BRICK_LEVELS", BRICK_LEVELS, |l| -> Flow {
            // The iv `L` read as a `uint` VALUE (the `dist_to_brick_exit` `p[a]` iv-as-value
            // discipline) — used in the `L >= pc.brick_levels` guard and the `(int)L` cast.
            let lu = C::iv_uint(l);
            // if (L >= pc.brick_levels) { break; }  — honor the runtime level count (a level >=
            // brick_levels is not active). `if_`'s `?` propagates the `brk`, which `runtime_for`
            // CONSUMES (the post-loop tail then runs).
            C::if_(C::uge(lu, pc_brick_levels()), C::brk)?;
            // float3 o = m2_levels[L].origin_brick_world.xyz;  — the level's lower world corner (a
            // NAMED `float3` temp; on Eval the value flows directly).
            let o = C::temp_vec3("o", level_vec3(l, "origin_brick_world.xyz"));
            // float bw = m2_levels[L].origin_brick_world.w;  — the level's brick world size (a
            // NAMED `float` temp).
            let bw = C::temp_float("bw", level_scalar(l, "origin_brick_world.w"));
            // float3 hi = o + m2_levels[L].dims_atlas_dim.xyz * bw;  — the level's upper corner.
            let hi = C::temp_vec3(
                "hi",
                C::vec3_add(o, C::vec3_mul_scalar(level_vec3(l, "dims_atlas_dim.xyz"), bw)),
            );
            // if (all(p >= o) && all(p < hi)) { return (int)L; }  — the containment hit: `p` is in
            // [o, hi) (the upper bound EXCLUSIVE — `p == hi` belongs to the next cell). `if_ret_i`'s
            // `?` forwards the `(int)L` return through `runtime_for` to the function IIFE.
            C::if_ret_i(
                ret_out,
                C::and2(C::all3_ge(p, o), C::all3_lt(p, hi)),
                C::int_from_uint(lu),
            )?;
            Flow::Continue(())
        })?;
        // return -1;  — outside all active levels (the caller folds the analytic field).
        C::ret_i(ret_out, C::int_lit_signed(-1))?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the early `?` already deposited the level index into the cell; on
    // Emit the recorder captured every statement.
    let _ = run();
}
