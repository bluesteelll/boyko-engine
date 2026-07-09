//! The M2 brick 3D-DDA cubic-hit MARCHER leaf (Increment 5c: the production
//! `m2_brick_cubic_hit` re-expressed in the control eDSL — the LARGEST + FINAL brick called body).
//!
//! `m2_brick_cubic_hit` (`sdf_gbuffer_composite.hlsl:1014`) marches `ro_v + rd_v·t` through a
//! SURFACE brick's interior voxel cells with a 3D-DDA, forms the JCGT cubic at the first cell whose
//! 8 corners bracket a sign change, and solves it for the in-cell crossing — returning the world
//! `t` of the FIRST hit (>= 0) or `-1.0` when the ray clears the brick without crossing. This
//! module authors the marcher body ONCE over the control-flow axis `C: Cf`, between the
//! `// === GENERATED m2_brick_cubic_hit BEGIN/END ===` sentinels INSIDE `m2_brick_cubic_hit`; the
//! hand-written signature, the `if (t_exit <= t_enter) { return -1.0; }` early-out, and the
//! `const uint W = M2_BRICK_ALLOC;` decl stay UN-generated ABOVE the BEGIN sentinel (framing (b),
//! keeping the generated span CONTIGUOUS so the `.contains()` sync pin holds).
//!
//! # EMIT-ONLY — no eval oracle
//!
//! The marcher calls `m2_corner(atlas, atlas_smp, ...)` → `atlas.SampleLevel(...)`, a `Texture3D`
//! the CPU cannot run. So `m2_brick_cubic_hit_body::<EvalCf>` is NEVER instantiated and there is NO
//! eval sweep: the cmp-`.spv` is the SOLE byte-identity gate (precedented by Inc 5a's
//! `unreachable!`-on-Eval level/pc hooks). EVERY new array/call/resource facet's `EvalCf` impl is
//! `unreachable!` (the honest-panic discipline, like [`Cf::call1`]). The body is generic over `C`
//! purely so the Emit recorder ([`crate::emit::emit_hlsl_m2_brick_cubic_hit`]) drives it.
//!
//! # The four new FACET GROUPS (Increment 5c)
//!
//!   1. NAMED LOCAL ARRAYS — `int cell[3]`, `int step[3]`, `float t_next[3]`, `float t_delta[3]` +
//!      a per-cell `float s[8]` corner buffer ([`Cf::decl_array_int`]/`_float`, the per-element
//!      get/set/`+=`). The `+=` is a DISTINCT statement ([`Cf::arr_int_add_assign`]) — the spike
//!      (R1) proved `cell[axis] += step[axis]` is NOT byte-identical to `cell[axis] = cell[axis] +
//!      step[axis]` at `-O0` (the `= +` form computes the access-chain TWICE).
//!   2. GENERALIZED CALL SITES — `m2_corner` (8 args incl. resource params, [`Cf::call_corner`]),
//!      `m2_jcgt_cubic_coeffs(s, ...)` (a by-name array arg, [`Cf::call_coeffs`]),
//!      `m2_marmitt_root` ([`Cf::call_marmitt`]), `(int)m2_clamp_index(g_entry)`
//!      ([`Cf::call_clamp_index_int`]).
//!   3. INT CASTS/ARITH — `(uint)max(cell[0], 0)` ([`Cf::smax`]/[`Cf::uint_from_int`]), `(float)(c0
//!      + 1)` ([`Cf::sadd`]/[`Cf::float_from_int`]), `W - 2u` ([`Cf::usub`]/[`Cf::captured_uint`]),
//!      `step[axis] == 0` ([`Cf::sint_eq`]), `cell[axis] < 0` ([`Cf::slt`]).
//!   4. MISC — the nested `uint` axis-select ([`Cf::select_uint`]), the dynamic `rd_v[axis]` index
//!      ([`Cf::vec3_dyn_index`]), the `float3(...)` scalar ctor ([`Cf::vec3_from_scalars`]), the
//!      captured `uint W` ([`Cf::captured_uint`]).
//!
//! # The 3-way `else if` REFLOW (`.spv`-neutral)
//!
//! The committed `if (rd_v[axis] > 0.0) {} else if (rd_v[axis] < 0.0) {} else {}` is authored as a
//! NESTED [`Cf::if_else`] (an `if_else` in the outer `else` arm), which prints `if (...) {} else {
//! if (...) {} else {} }`. This is `.spv`-neutral — `else if` IS `else { if }` grammatically (the
//! wrapping block adds only an empty scope), proven by the cmp-`.spv`; the committed source is
//! re-spliced to the `else { if }` text so the `.contains()` sync pin matches.
//!
//! # The loop is `runtime_for`, NOT `unroll_for`
//!
//! Both loops use [`Cf::runtime_for`]: the setup `[unroll]` (bound literal `"3u"`) and the DDA
//! `[loop]` (bound symbol `"M2_MAX_CELLS"`). The DDA loop has an in-loop early `return seg_lo +
//! local_t;` ([`Cf::if_ret_f`]) that must FORWARD through the loop to the function-scope IIFE's `?`
//! — which `runtime_for`'s `Break(Return)` arm does and `unroll_for`'s does not. The two `break`s
//! (`t_cell_exit >= t_exit`, the DDA-exit guard) are [`Cf::brk`], CONSUMED by `runtime_for`.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The apron'd-grid coordinate shift's APRON term (`+ M2_APRON`) — one voxel. Mirrors the GPU's
/// `M2_APRON` (`sdf_gbuffer_composite.hlsl:662`). Spelled SYMBOLICALLY in the emitted HLSL
/// (`M2_APRON`, NOT `1.0`).
pub const M2_APRON: f32 = 1.0;

/// The atlas-sample bias (`+ M2_ATLAS_BIAS`) — golden-locked to 0. Mirrors the GPU's
/// `M2_ATLAS_BIAS` (`sdf_gbuffer_composite.hlsl:663`). Spelled SYMBOLICALLY (`M2_ATLAS_BIAS`, NOT
/// `0.0`).
pub const M2_ATLAS_BIAS: f32 = 0.0;

/// The longest 3D-DDA path (`3 * BRICK_ALLOC`) — the DDA `[loop]` trip count, the BOUND SYMBOL the
/// for-header carries. Mirrors the GPU's `M2_MAX_CELLS` (`sdf_gbuffer_composite.hlsl:666`). Spelled
/// SYMBOLICALLY in the emitted HLSL header (`M2_MAX_CELLS`, NOT `30u`).
pub const M2_MAX_CELLS: usize = 30;

/// Marches `ro_v + rd_v·t` through the brick's interior voxel cells (3D-DDA), forming + solving the
/// JCGT cubic at the first sign-bracketing cell, depositing the world `t` of the first hit (>= 0)
/// or `-1.0` (no crossing) into `ret_out`. Authored ONCE over the control-flow axis `C`. Mirrors the
/// GPU `m2_brick_cubic_hit`'s L1021-1102 statement-for-statement (the hand-written signature, the
/// `t_exit <= t_enter` early-out, and the `const uint W` decl stay un-generated ABOVE the span).
///
/// `atlas`/`atlas_smp` are the level's brick atlas + sampler (RESOURCE params, [`Cf::ResTok`]);
/// `ro_v`/`rd_v` the ray in interior-voxel units (WHOLE `float3` params — indexed `rd_v[axis]` AND
/// passed whole to `m2_jcgt_cubic_coeffs`); `t_enter`/`t_exit` the brick-span bounds; `tile_org` the
/// atlas-voxel tile origin; `inv_atlas`/`band_half` the corner-decode scalars; `w` the captured
/// `uint W` (= `M2_BRICK_ALLOC`, declared by the hand-written shader above the span).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; ret_f(ret_out, -1.0)?; Continue }`, so
/// the in-loop early `return seg_lo + local_t;` ([`Cf::if_ret_f`]) forwards through
/// [`Cf::runtime_for`]'s `?` to the IIFE's `?` — skipping the tail `return -1.0;`. The two
/// `break`s are [`Cf::brk`] propagated through [`Cf::if_`]'s `?`; `runtime_for` CONSUMES them, so
/// the post-loop tail runs. On Emit every branch is recorded structurally (`?` never early-returns);
/// the whole marcher is captured. EMIT-ONLY: never instantiated over `EvalCf`.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn m2_brick_cubic_hit_body<C: Cf>(
    atlas: C::ResTok,
    atlas_smp: C::ResTok,
    ro_v: C::Vec3f,
    rd_v: C::Vec3f,
    tile_org: C::Vec3f,
    t_enter: C::Scalar,
    t_exit: C::Scalar,
    inv_atlas: C::Scalar,
    band_half: C::Scalar,
    w: C::Uint,
    ret_out: &C::RetCellF,
) {
    type S<C> = <C as Cf>::Scalar;

    // The `+ M2_APRON - 0.5 + M2_ATLAS_BIAS` apron-shift terms (SYMBOLS / a literal), shared by the
    // `g_entry` setup and the `lo_g` cell-local ctor. `apron`/`bias` are named-lit SYMBOLS; the `0.5`
    // is a plain literal. The `0.5`/`M2_APRON`/`M2_ATLAS_BIAS` recompute per-use (no `temp`), so each
    // use spells inline — matching the committed body, which repeats the shift text at each site.
    let apron = || C::named_lit("M2_APRON", M2_APRON);
    let bias = || C::named_lit("M2_ATLAS_BIAS", M2_ATLAS_BIAS);
    let half = || S::<C>::lit(0.5);

    let run = || -> Flow {
        // float t = t_enter;  — the carried march `t` (reassigned in the DDA loop, so a `decl_var`).
        let t = C::decl_var("t", t_enter);

        // int cell[3]; int step[3]; float t_next[3]; float t_delta[3];  — the four UNINITIALIZED DDA
        // state arrays (filled in the `[unroll]` setup loop below).
        let cell = C::decl_array_int("cell", 3);
        let step = C::decl_array_int("step", 3);
        let t_next = C::decl_array_float("t_next", 3);
        let t_delta = C::decl_array_float("t_delta", 3);

        // [unroll] for (uint axis = 0u; axis < 3u; ++axis) { ... }  — the per-axis DDA setup. The
        // body has no early return, but it reuses `runtime_for` (the LITERAL bound `"3u"`, matching
        // `m2_brick_span`'s axis loop) for the uniform `for (uint axis = 0u; axis < 3u; ++axis)`
        // header text.
        C::runtime_for("[unroll]", "axis", "3u", 3, |axis_iv| -> Flow {
            // The iv `axis` as a `uint` VALUE (the `dist_to_brick_exit` iv-as-value discipline) —
            // used both as a dynamic float3 index (`rd_v[axis]`) and as an array subscript
            // (`cell[axis]`). On Emit it is the SAME `UintInput` node spelling `axis`.
            let axis = C::iv_uint(axis_iv);
            // float g_entry = ro_v[axis] + rd_v[axis] * t + M2_APRON - 0.5 + M2_ATLAS_BIAS;  — the
            // apron'd-grid coordinate at entry. `ro_v[axis]`/`rd_v[axis]` are dynamic float3-param
            // indexes; the left-associative additive chain prints flat.
            let ro_a = C::vec3_dyn_index(ro_v, axis);
            let rd_a = C::vec3_dyn_index(rd_v, axis);
            let g_entry = C::temp_float(
                "g_entry",
                ro_a
                    .add(rd_a.mul(C::get_var(&t)))
                    .add(apron())
                    .sub(half())
                    .add(bias()),
            );
            // int c0 = (int)m2_clamp_index(g_entry);  — the floored low cell index, signed (so the
            // `(float)(c0 + 1)` boundary + the `step[axis]` add are signed). A NAMED `int` temp via
            // the call+cast facet.
            let c0 = C::temp_int("c0", C::call_clamp_index_int("m2_clamp_index", g_entry));
            // cell[axis] = c0;
            C::arr_int_set(cell, axis, c0);

            // if (rd_v[axis] > 0.0) { ... } else if (rd_v[axis] < 0.0) { ... } else { ... }  — the
            // per-axis DDA direction setup. Authored as a NESTED `if_else` (the `else if` reflow):
            // the outer `else` arm is itself an `if_else`, printing `if (...) {} else { if (...) {}
            // else {} }` (`.spv`-neutral — `else if` IS `else { if }`).
            C::if_else(
                C::vec3_dyn_index(rd_v, axis).gt(S::<C>::lit(0.0)),
                || -> Flow {
                    // step[axis] = 1;
                    C::arr_int_set(step, axis, C::int_lit_signed(1));
                    // float boundary = (float)(c0 + 1);  — reads the LOCAL `c0`, NOT `cell[axis]`.
                    let boundary =
                        C::temp_float("boundary", C::float_from_int(C::sadd(c0, C::int_lit_signed(1))));
                    // t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
                    C::arr_float_set(
                        t_next,
                        axis,
                        C::get_var(&t).add(
                            boundary
                                .sub(g_entry)
                                .div(C::vec3_dyn_index(rd_v, axis)),
                        ),
                    );
                    // t_delta[axis] = 1.0 / rd_v[axis];
                    C::arr_float_set(
                        t_delta,
                        axis,
                        S::<C>::lit(1.0).div(C::vec3_dyn_index(rd_v, axis)),
                    );
                    Flow::Continue(())
                },
                || -> Flow {
                    C::if_else(
                        C::vec3_dyn_index(rd_v, axis).lt(S::<C>::lit(0.0)),
                        || -> Flow {
                            // step[axis] = -1;
                            C::arr_int_set(step, axis, C::int_lit_signed(-1));
                            // float boundary = (float)c0;  — reads the LOCAL `c0`, NOT `cell[axis]`.
                            let boundary = C::temp_float("boundary", C::float_from_int(c0));
                            // t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
                            C::arr_float_set(
                                t_next,
                                axis,
                                C::get_var(&t).add(
                                    boundary
                                        .sub(g_entry)
                                        .div(C::vec3_dyn_index(rd_v, axis)),
                                ),
                            );
                            // t_delta[axis] = -1.0 / rd_v[axis];
                            C::arr_float_set(
                                t_delta,
                                axis,
                                S::<C>::lit(-1.0).div(C::vec3_dyn_index(rd_v, axis)),
                            );
                            Flow::Continue(())
                        },
                        || -> Flow {
                            // step[axis] = 0;
                            C::arr_int_set(step, axis, C::int_lit_signed(0));
                            // t_next[axis] = 1.0e30;
                            C::arr_float_set(t_next, axis, S::<C>::lit(1.0e30));
                            // t_delta[axis] = 1.0e30;
                            C::arr_float_set(t_delta, axis, S::<C>::lit(1.0e30));
                            Flow::Continue(())
                        },
                    )
                },
            )?;
            Flow::Continue(())
        })?;

        // [loop] for (uint iter = 0u; iter < M2_MAX_CELLS; ++iter) { ... }  — the 3D-DDA march. The
        // BOUND SYMBOL `M2_MAX_CELLS` (NOT `30u`); the in-loop early `return seg_lo + local_t;`
        // forwards through this `runtime_for` to the function IIFE.
        C::runtime_for("[loop]", "iter", "M2_MAX_CELLS", M2_MAX_CELLS, |_iter| -> Flow {
            // uint cx = min((uint)max(cell[0], 0), W - 2u);  (cy/cz mirror) — the cell's low corner,
            // clamped so the +1 neighbour is in-bounds. `max(cell[0], 0)` is a SIGNED max, `(uint)`
            // the int->uint cast, `W - 2u` the captured-`uint` subtract.
            let w2 = || C::usub(w, C::uint_lit(2));
            let lit0 = || C::int_lit_signed(0);
            let cx = C::temp_uint(
                "cx",
                C::umin(
                    C::uint_from_int(C::smax(C::arr_int_get(cell, C::uint_lit(0)), lit0())),
                    w2(),
                ),
            );
            let cy = C::temp_uint(
                "cy",
                C::umin(
                    C::uint_from_int(C::smax(C::arr_int_get(cell, C::uint_lit(1)), lit0())),
                    w2(),
                ),
            );
            let cz = C::temp_uint(
                "cz",
                C::umin(
                    C::uint_from_int(C::smax(C::arr_int_get(cell, C::uint_lit(2)), lit0())),
                    w2(),
                ),
            );

            // float s[8];  — the per-cell corner buffer (fetched below in s_ijk ↔ x + 2y + 4z order).
            let s = C::decl_array_float("s", 8);
            // s[k] = m2_corner(atlas, atlas_smp, tile_org, cx(+1u), cy(+1u), cz(+1u), inv_atlas,
            // band_half);  — the 8 corners. The `cx + 1u` neighbours reuse `uadd(c, 1u)`.
            let one_u = || C::uint_lit(1);
            let cxp = || C::uadd(cx, one_u());
            let cyp = || C::uadd(cy, one_u());
            let czp = || C::uadd(cz, one_u());
            let corner = |k: u32, ix: C::Uint, iy: C::Uint, iz: C::Uint| {
                C::arr_float_set(
                    s,
                    C::uint_lit(k),
                    C::call_corner(
                        "m2_corner", atlas, atlas_smp, tile_org, ix, iy, iz, inv_atlas, band_half,
                    ),
                );
            };
            corner(0, cx, cy, cz); // s000
            corner(1, cxp(), cy, cz); // s100
            corner(2, cx, cyp(), cz); // s010
            corner(3, cxp(), cyp(), cz); // s110
            corner(4, cx, cy, czp()); // s001
            corner(5, cxp(), cy, czp()); // s101
            corner(6, cx, cyp(), czp()); // s011
            corner(7, cxp(), cyp(), czp()); // s111

            // float t_cell_exit = min(min(min(t_next[0], t_next[1]), t_next[2]), t_exit);  — this
            // cell's far-side t along the ray (clamped to the brick span).
            let tn = |i: u32| C::arr_float_get(t_next, C::uint_lit(i));
            let t_cell_exit = C::temp_float(
                "t_cell_exit",
                tn(0).min(tn(1)).min(tn(2)).min(t_exit),
            );
            // float seg_lo = max(t, t_enter);  float seg_hi = min(t_cell_exit, t_exit);
            let seg_lo = C::temp_float("seg_lo", C::get_var(&t).max(t_enter));
            let seg_hi = C::temp_float("seg_hi", t_cell_exit.min(t_exit));

            // if (seg_hi > seg_lo) { ... }  — form + solve the cubic in this cell's t-segment.
            C::if_(seg_hi.gt(seg_lo), || -> Flow {
                // float3 lo_g = float3(ro_v[i] + rd_v[i] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS -
                // (float)c{x,y,z}, ...);  — the ray origin in the cell's LOCAL [0,1]³ frame.
                let comp = |i: u32, c: C::Uint| -> C::Scalar {
                    C::vec3_dyn_index(ro_v, C::uint_lit(i))
                        .add(C::vec3_dyn_index(rd_v, C::uint_lit(i)).mul(seg_lo))
                        .add(apron())
                        .sub(half())
                        .add(bias())
                        .sub(C::float_from_uint(c))
                };
                let lo_g = C::temp_vec3(
                    "lo_g",
                    C::vec3_from_scalars(comp(0, cx), comp(1, cy), comp(2, cz)),
                );
                // float4 coeffs = m2_jcgt_cubic_coeffs(s, lo_g, rd_v);
                let coeffs = C::temp_vec4("coeffs", C::call_coeffs("m2_jcgt_cubic_coeffs", s, lo_g, rd_v));
                // float local_t = m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo);
                let local_t = C::temp_float(
                    "local_t",
                    C::call_marmitt("m2_marmitt_root", coeffs, S::<C>::lit(0.0), seg_hi.sub(seg_lo)),
                );
                // if (local_t >= 0.0) { return seg_lo + local_t; }  — the in-cell crossing found;
                // `if_ret_f`'s `?` forwards the return through `runtime_for` to the function IIFE.
                C::if_ret_f(ret_out, local_t.ge(S::<C>::lit(0.0)), seg_lo.add(local_t))?;
                Flow::Continue(())
            })?;

            // if (t_cell_exit >= t_exit) { break; }  — past the brick exit, stop. `if_`'s `?`
            // propagates the `brk`, which `runtime_for` CONSUMES (the post-loop tail runs).
            C::if_(t_cell_exit.ge(t_exit), C::brk)?;

            // uint axis = (t_next[0] <= t_next[1] && t_next[0] <= t_next[2]) ? 0u : ((t_next[1] <=
            // t_next[2]) ? 1u : 2u);  — the nearest-boundary axis (the nested `uint` select).
            let axis = C::temp_uint(
                "axis",
                C::select_uint(
                    C::and2(tn(0).le(tn(1)), tn(0).le(tn(2))),
                    C::uint_lit(0),
                    C::select_uint(tn(1).le(tn(2)), C::uint_lit(1), C::uint_lit(2)),
                ),
            );
            // t = t_next[axis];  — advance the march `t` to that boundary.
            C::set_var(&t, C::arr_float_get(t_next, axis));
            // cell[axis] += step[axis];  — step the cell (the `+=` TOKEN, R1).
            C::arr_int_add_assign(cell, axis, C::arr_int_get(step, axis));
            // t_next[axis] += t_delta[axis];  — advance that axis's next boundary (the `+=` TOKEN).
            C::arr_float_add_assign(t_next, axis, C::arr_float_get(t_delta, axis));
            // if (step[axis] == 0 || cell[axis] < 0 || (uint)cell[axis] >= W - 1u) { break; }  — the
            // DDA-exit guard (a parallel axis / out the low or high face). `step[axis] == 0` is a
            // signed `==`, `cell[axis] < 0` a signed `<`, `(uint)cell[axis] >= W - 1u` a uint `>=`.
            let exit = C::or(
                C::or(
                    C::sint_eq(C::arr_int_get(step, axis), C::int_lit_signed(0)),
                    C::slt(C::arr_int_get(cell, axis), C::int_lit_signed(0)),
                ),
                C::uge(
                    C::uint_from_int(C::arr_int_get(cell, axis)),
                    C::usub(w, C::uint_lit(1)),
                ),
            );
            C::if_(exit, C::brk)?;
            Flow::Continue(())
        })?;

        // return -1.0;  — the ray cleared the brick without a crossing.
        C::ret_f(ret_out, S::<C>::lit(-1.0))?;
        Flow::Continue(())
    };
    // Discard the Flow: on Emit the recorder captured every statement (the body is EMIT-ONLY).
    let _ = run();
}
