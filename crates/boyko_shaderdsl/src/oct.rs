//! The G-buffer OCTAHEDRAL-NORMAL ENCODER leaf (Track B Increment G2: the LAST G-buffer leaf — it
//! lands the mutable `float3`/`float2` locals + the `float2` component ops).
//!
//! `oct_encode` (`sdf_gbuffer_composite.hlsl:507`) folds a UNIT normal `n` into a `[0,1]^2`
//! octahedral pair stored in the RG channels of the RGBA8 `gNormal` target (the resolve decodes it via
//! `oct_decode`). This module authors the BODY ONCE over the control-flow axis `C: Cf`, between the
//! `// === GENERATED oct_encode BEGIN/END ===` sentinels INSIDE `oct_encode`; the hand-written
//! signature `float2 oct_encode(float3 n) {` + the closing `}` stay un-generated (framing (b)).
//!
//! # The new facets — the mutable locals + the `float2` component ops
//!
//! Unlike the prior G-buffer leaves, `oct_encode` MUTATES its inputs in place. `n` is a
//! SUPPRESSED-DECL mutable `float3` (the signature PARAMETER, reassigned by `n /= ...`), modeled as
//! [`Cf::decl_param_vec3`] (the `float3` analogue of the marcher's scalar carried param) +
//! [`Cf::set_var_vec3`]; the committed `n /= (...)` is emitted in the proven byte-identical R1 form
//! `n = n / (...)` (a whole-variable op, NO access chain — the R1 spike). `e` is a TRUE mutable
//! `float2` local (`float2 e = n.xy;`, reassigned inside the `if`), modeled as [`Cf::decl_var_vec2`] +
//! [`Cf::set_var_vec2`] (the `float2` analogue of [`Cf::decl_var`]).
//!
//! The `if (n.z < 0.0)` is a REAL fall-through [`Cf::if_`] (an `OpBranchConditional`+merge — forcing it
//! branchless would emit an `OpSelect` and FORK the `.spv`). The two inner sign-ternaries (`e.x >= 0.0
//! ? 1.0 : -1.0`) are scalar [`Cf::select_bare`]s (the BARE ternary — the committed spelling wraps
//! NOTHING, DISTINCT from `Cf::select`'s both-arms-wrapped regula-falsi form). The fused `e * 0.5 +
//! 0.5` return is ONE expression (a
//! split `e *= 0.5; e += 0.5` would fork the `.spv` — an extra store/load).
//!
//! # Instantiation (the established control-axis discipline)
//!
//! - `<EvalCf>` — the CPU oracle (real `/`/`abs`/`*`/`+`/branch + a `Cell<[f32; 3]>` for `n` and a
//!   `Cell<[f32; 2]>` for `e`). The eval sweep reproduces the committed encode to-bits against a host
//!   mirror transcribing the committed body verbatim, over BOTH hemispheres (lower `n.z < 0` and upper
//!   `n.z >= 0`, exercising the `if` both ways) and all four sign quadrants (both sign-ternaries).
//! - `<EmitCf>` — the HLSL recorder; the printer ([`crate::emit::emit_hlsl_oct_encode`]) walks the
//!   STMT IR into the body span (byte-identical to the committed `.comp.spv`, proven by the cmp-`.spv`).
//!
//! # `R1` (the `n /= ...` → `n = n / ...` whole-variable form)
//!
//! The committed source's `n /= (...)` and the emitted `n = n / (...)` are BYTE-IDENTICAL at the
//! `.spv` level (the R1 spike: `n` is a whole variable, so the `/=` is a load+div+store with no
//! access chain — the same SPIR-V the explicit `n = n / (...)` lowers to). The committed source is
//! re-spliced to the `n = n / (...)` spelling; the `.comp.spv` stays byte-identical.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The octahedral wrap pivot — `n.z < 0.0` selects the lower hemisphere's fold. Mirrors the GPU's
/// literal `0.0` (`sdf_gbuffer_composite.hlsl:510`).
const HEMISPHERE_PIVOT: f32 = 0.0;

// ---- oct_decode: the DDGI-tile octahedral DECODE (the inverse of oct_encode) ------------------

/// The `[0,1] -> [-1,1]` unfold scale — `e * 2.0 - 1.0`. Mirrors the GPU `oct_decode`'s literal
/// `2.0` (`deferred_pbr.hlsl:398`). The exact inverse of [`REMAP_SCALE`]/[`REMAP_BIAS`].
const UNFOLD_SCALE: f32 = 2.0;

/// The `[0,1] -> [-1,1]` unfold bias — `... - 1.0`. Mirrors the GPU `oct_decode`'s literal `1.0`.
const UNFOLD_BIAS: f32 = 1.0;

/// The `nz = 1.0 - abs(nx) - abs(ny)` reconstruction constant. Mirrors the GPU `oct_decode`'s
/// literal `1.0` (`deferred_pbr.hlsl:399`).
const RECONSTRUCT_ONE: f32 = 1.0;

/// The lower-hemisphere sign-fold comparand — `nx >= 0.0` chooses `-t`/`+t`. Mirrors the GPU
/// `oct_decode`'s literal `0.0` (`deferred_pbr.hlsl:401-402`).
const FOLD_SIGN_PIVOT: f32 = 0.0;

/// The sign-ternary comparand — `e.x >= 0.0` chooses `+1`/`-1`. Mirrors the GPU's literal `0.0`
/// (`sdf_gbuffer_composite.hlsl:511`).
const SIGN_PIVOT: f32 = 0.0;

/// The `[-1,1] -> [0,1]` remap scale — `e * 0.5 + 0.5`. Mirrors the GPU's literal `0.5`
/// (`sdf_gbuffer_composite.hlsl:513`).
const REMAP_SCALE: f32 = 0.5;

/// The `[-1,1] -> [0,1]` remap bias — `... + 0.5`. Mirrors the GPU's literal `0.5`.
const REMAP_BIAS: f32 = 0.5;

/// The positive sign-ternary arm — `e.x >= 0.0 ? 1.0 : ...`. Mirrors the GPU's literal `1.0`.
const SIGN_POS: f32 = 1.0;

/// The negative sign-ternary arm — `... ? ... : -1.0`. Mirrors the GPU's literal `-1.0`.
const SIGN_NEG: f32 = -1.0;

/// The `1.0 - abs(e.yx)` mirror constant. Mirrors the GPU's literal `1.0`
/// (`sdf_gbuffer_composite.hlsl:511`).
const MIRROR_ONE: f32 = 1.0;

/// Octahedral-encodes a UNIT normal `n` into a `[0,1]^2` pair, depositing it into `ret_out`. Authored
/// ONCE over the control-flow axis `C`. Mirrors the GPU `oct_encode`'s L508-513 body
/// statement-for-statement (the hand-written signature + closing brace stay un-generated).
///
/// On Emit the body records `n = n / (abs(n.x) + abs(n.y) + abs(n.z));` (a bare suppressed-decl
/// assign), `float2 e = n.xy;` (a `float2` local decl), the `if (n.z < 0.0) { e = (1.0 - abs(e.yx)) *
/// float2(e.x >= 0.0 ? 1.0 : -1.0, e.y >= 0.0 ? 1.0 : -1.0); }` fall-through branch, and the fused
/// `return e * 0.5 + 0.5;`. On Eval the body runs the real `/`/`abs`/branch/`*`/`+` and deposits the
/// `[0,1]^2` pair into the cell.
#[inline]
pub fn oct_encode_body<C: Cf>(n_param: C::Vec3f, ret_out: &C::RetCellV2) -> Flow {
    // n /= (abs(n.x) + abs(n.y) + abs(n.z));  — the L1-normalize onto the octahedron (the R1 whole-
    // variable form `n = n / (...)`; `n` is the suppressed-decl mutable param).
    let n = C::decl_param_vec3("n", n_param);
    let abs_sum = C::vec3_x(C::get_var_vec3(&n))
        .abs()
        .add(C::vec3_y(C::get_var_vec3(&n)).abs())
        .add(C::vec3_z(C::get_var_vec3(&n)).abs());
    C::set_var_vec3(&n, C::vec3_div_scalar(C::get_var_vec3(&n), abs_sum));

    // float2 e = n.xy;  — the projected XY (a mutable `float2` local).
    let e = C::decl_var_vec2("e", C::vec3_xy(C::get_var_vec3(&n)));

    // if (n.z < 0.0) { e = (1.0 - abs(e.yx)) * float2(e.x >= 0.0 ? 1.0 : -1.0, e.y >= 0.0 ? 1.0 :
    // -1.0); }  — the lower-hemisphere fold (a REAL fall-through branch; the body reassigns `e`).
    C::if_(
        C::vec3_z(C::get_var_vec3(&n)).lt(C::Scalar::lit(HEMISPHERE_PIVOT)),
        || -> Flow {
            let folded = C::vec2_mul(
                C::vec2_rsub_scalar(
                    C::Scalar::lit(MIRROR_ONE),
                    C::vec2_abs(C::vec2_yx(C::get_var_vec2(&e))),
                ),
                C::vec2_from_scalars(
                    C::select_bare(
                        C::vec2_x(C::get_var_vec2(&e)).ge(C::Scalar::lit(SIGN_PIVOT)),
                        C::Scalar::lit(SIGN_POS),
                        C::Scalar::lit(SIGN_NEG),
                    ),
                    C::select_bare(
                        C::vec2_y(C::get_var_vec2(&e)).ge(C::Scalar::lit(SIGN_PIVOT)),
                        C::Scalar::lit(SIGN_POS),
                        C::Scalar::lit(SIGN_NEG),
                    ),
                ),
            );
            C::set_var_vec2(&e, folded);
            Flow::Continue(())
        },
    )?;

    // return e * 0.5 + 0.5;  — the [-1,1] -> [0,1] UNORM remap (ONE fused expression).
    C::ret_vec2(
        ret_out,
        C::vec2_add_scalar(
            C::vec2_mul_scalar(C::get_var_vec2(&e), C::Scalar::lit(REMAP_SCALE)),
            C::Scalar::lit(REMAP_BIAS),
        ),
    )
}

/// Octahedral-DECODES a `[0,1]^2` tile UV pair `(ex, ey)` into the PRE-NORMALIZE direction
/// lanes `[nx, ny, nz]` — the mathematical inverse of [`oct_encode_body`]. Authored ONCE over
/// the control-flow axis `C`, host-mirrorable (its `<EvalCf>` instantiation, after `normalize`,
/// equals the [`oct_encode_body`] round-trip and the I0b host mirror `goldens::oct_decode`).
///
/// I2 is the FIRST rung to commit a DDGI-tile `oct_decode` (the SDFDDGI plan §1.4 P0-2 fix).
/// This body single-sources it: the I2 update pass weights cached rays against each texel's
/// decoded direction (`texelDir = oct_decode(texelUV)`), and I3's resolve MUST decode against
/// this EXACT eDSL-emitted body. The three lanes are returned PRE-`normalize` (the emitter
/// [`crate::emit::emit_hlsl_oct_decode`] wraps the tail `return normalize(float3(nx, ny, nz));`
/// textually — the `sdf_normal` normalize-as-text precedent, since there is no `Vec3Normalize`
/// emit node, so no frozen `.spv` can fork).
///
/// # The I2 → I3 decode contract (load-bearing)
///
/// The tile-UV↔texel REMAP and probe-spacing reconstruction that I3 owns MUST live in the
/// texel→UV chain OUTSIDE this decode, never inside it — else I2's per-texel WRITE iteration
/// and I3's per-sample READ desync (the silent point-cube-drift class). This body takes an
/// ALREADY-remapped `[0,1]^2` tile-UV pair and inverts ONLY the octahedral fold; the host
/// mirror `goldens::oct_decode` has the identical boundary (it too takes `e` = a `[0,1]^2`
/// pair). The `oct_decode_edsl_matches_host` sync test pins the two equal.
///
/// The committed math (matching `deferred_pbr.hlsl:397-404` / `goldens::oct_decode`):
/// ```text
/// e  = e * 2.0 - 1.0;                              // [0,1] -> [-1,1]
/// n  = float3(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
/// t  = saturate(-n.z);                             // = clamp(-nz, 0, 1)
/// n.x += n.x >= 0.0 ? -t : t;
/// n.y += n.y >= 0.0 ? -t : t;
/// return normalize(n);
/// ```
#[inline]
pub fn oct_decode_body<C: Cf>(ex: C::Scalar, ey: C::Scalar) -> [C::Scalar; 3] {
    // e = e * 2.0 - 1.0;  — the [0,1] -> [-1,1] unfold (materialized as `ex`/`ey` temps so the
    // reconstruct + fold read the SAME unfolded value, not a recomputed `ex*2-1`).
    let unfold_scale = C::Scalar::lit(UNFOLD_SCALE);
    let unfold_bias = C::Scalar::lit(UNFOLD_BIAS);
    let ex = C::temp_float("ex", ex.mul(unfold_scale).sub(unfold_bias));
    let ey = C::temp_float("ey", ey.mul(unfold_scale).sub(unfold_bias));

    // float3 n = float3(ex, ey, 1.0 - abs(ex) - abs(ey));  — carried as the three lanes
    // `nx`/`ny`/`nz` (the fold reassigns `nx`/`ny`; `nz` is read once for `t`).
    let nx0 = ex;
    let ny0 = ey;
    let nz = C::temp_float(
        "nz",
        C::Scalar::lit(RECONSTRUCT_ONE).sub(ex.abs()).sub(ey.abs()),
    );

    // float t = saturate(-nz);  — the lower-hemisphere fold amount (clamp of the negated z).
    let t = C::temp_float("t", nz.neg().clamp01());

    // n.x += n.x >= 0.0 ? -t : t;  — the sign-mirroring fold (a bare sign-ternary, the
    // `oct_encode` `select_bare` shape). Spelled as `nx = nx + (nx >= 0 ? -t : t)`.
    let pivot = C::Scalar::lit(FOLD_SIGN_PIVOT);
    let nx = C::temp_float("nx", nx0.add(C::select_bare(nx0.ge(pivot), t.neg(), t)));
    // n.y += n.y >= 0.0 ? -t : t;
    let ny = C::temp_float("ny", ny0.add(C::select_bare(ny0.ge(pivot), t.neg(), t)));

    // return normalize(float3(nx, ny, nz));  — the emitter wraps the `normalize(...)` textually.
    [nx, ny, nz]
}
