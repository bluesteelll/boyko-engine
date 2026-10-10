//! The brick-atlas `R8_SNORM` decode leaves, authored ONCE generic over
//! [`FieldScalar`] (A2 — the first INTEGER/bit leaf).
//!
//! [`decode_snorm8`] is the inverse of the `fill_brick` snorm encode: a stored
//! narrow-band code `q ∈ [-128, 127]` maps onto a world distance. Mirroring the
//! frozen reference operand-for-operand:
//! `boyko_sdf_math::brick::decode_snorm8` (the CPU oracle) AND the GPU brick fetch in
//! `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl` (`m2_decode`).
//!
//! # The decode is SPLIT across CPU and GPU — only the scale is shader code
//!
//! On the GPU the `R8_SNORM` atlas is a 3D texture: the FIXED-FUNCTION sampler
//! performs the byte → normalized-float step (`q → n = max(q/127, -1)`, including the
//! `-128 → -1` snorm asymmetry) in hardware, and the shader's `m2_decode(n, band_half)`
//! only applies the world scale `n * band_half`. The host `decode_snorm8(q, band_half)`
//! does BOTH steps in one function (no hardware sampler on the CPU oracle).
//!
//! So this module authors the leaf in two pieces:
//! - [`snorm_normalize`] — the byte → `n` step (`q == i8::MIN ? -1 : q/127`). It is the
//!   part the GPU does in HARDWARE, so it is CPU-only: the `Emit` instantiation is
//!   never spliced (the sampler, not shader code, performs it).
//! - [`snorm_scale`] — the world scale `n * band_half`. This IS `m2_decode`: a pure
//!   `float` op that single-sources to the shader. Its `Emit` instantiation is the
//!   body spliced between the `// === GENERATED decode_snorm8 BEGIN/END ===` sentinels.
//! - [`decode_snorm8`] — the full `snorm_scale(snorm_normalize(q), band_half)`. Its
//!   `f32` Eval instantiation is byte-identical to the host `decode_snorm8`; this is the
//!   CPU authority the `eval_byte_identity` to-bits sweep locks.
//!
//! # `no_std`
//!
//! `#![no_std]`-clean (the Eval path is a leaf, like [`crate::field`]). The integer
//! ops ([`FieldScalar::int_lit`] / [`FieldScalar::int_eq`] /
//! [`FieldScalar::int_to_float`]) lower to single `core` `i32`/`f32` instructions on
//! the `f32` backend.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// `BrickClass::EmptyOutside as u32` is grid-content, but the OUT-OF-GRID sentinel the
/// host `host_brick_cell` returns as `None` and the GPU `brick_cell_class` returns
/// directly is `0xFFFFFFFF`. Mirrors the shader's `BRICK_OUTSIDE_GRID`
/// (`sdf_gbuffer_composite.hlsl:558`). Spelled SYMBOLICALLY in the emitted HLSL (the
/// committed body writes `BRICK_OUTSIDE_GRID`, not `4294967295u`).
pub const BRICK_OUTSIDE_GRID: u32 = 0xFFFF_FFFF;

/// The `R8_SNORM` normalize divisor — `q ∈ [-127, 127]` maps onto `[-1, 1]` as
/// `q / 127`. Mirrors the host `decode_snorm8`'s `127.0` (brick.rs:1052) and the
/// Vulkan `R8_SNORM` rule.
pub const SNORM_DIVISOR: f32 = 127.0;

/// The minimum per-step progress a brick-exit makes (world units) — the empty-skip
/// PROGRESS GUARANTEE. Mirrors `boyko_sdf_math::brick::BRICK_EXIT_EPS` (brick.rs:988)
/// and the GPU shader's `BRICK_EXIT_EPS` (`sdf_gbuffer_composite.hlsl:553`). A
/// face-parallel / boundary-grazing ray computes a zero/negative exit and would stall;
/// clamping UP to this forces the march forward. Spelled SYMBOLICALLY in the emitted
/// HLSL (the committed body writes `BRICK_EXIT_EPS`, not `1.0e-4`).
pub const BRICK_EXIT_EPS: f32 = 1.0e-4;

/// The snorm sentinel code: `i8::MIN` (-128). The asymmetric `R8_SNORM` rule maps it
/// (and -127) to `-1.0`. Mirrors the host `decode_snorm8`'s `i8::MIN` branch.
pub const SNORM_SENTINEL: i32 = i8::MIN as i32;

/// The fixed regula-falsi iteration budget — `m2_regula_falsi`'s `[loop]` trip count.
/// Mirrors the GPU's `M2_MARMITT_ITERS` (`sdf_gbuffer_composite.hlsl:656`) and the host
/// `boyko_sdf_math::brick::MARMITT_ITERS` (brick.rs:1276). Spelled SYMBOLICALLY in the
/// emitted HLSL header (`M2_MARMITT_ITERS`, NOT `8u`) — the bound symbol the `[loop]`
/// for-header carries.
pub const M2_MARMITT_ITERS: usize = 8;

/// The root residual / bracket-collapse tolerance — `m2_regula_falsi`'s early-return
/// guard `abs(f_mid) <= M2_CUBIC_ROOT_EPS || (hi - lo) <= M2_CUBIC_ROOT_EPS`. Mirrors the
/// GPU's `M2_CUBIC_ROOT_EPS` (`sdf_gbuffer_composite.hlsl:655`) and the host
/// `boyko_sdf_math::brick::CUBIC_ROOT_EPS` (brick.rs:1269). Spelled SYMBOLICALLY in the
/// emitted HLSL (`M2_CUBIC_ROOT_EPS`, NOT `1.0e-6`).
pub const M2_CUBIC_ROOT_EPS: f32 = 1.0e-6;

/// The degenerate-bracket guard `abs(denom) > M2_REGULA_DENOM_EPS` — when the function
/// values at the bracket ends are too close (a flat secant), `m2_regula_falsi` falls back
/// to the bisection midpoint. The committed GPU body spells the LITERAL `1.0e-30` inline
/// (NOT a named symbol); this constant is the eDSL single-source of that literal. NOTE:
/// the GPU shape uses `1.0e-30`, whereas the host `regula_falsi` uses `f32::MIN_POSITIVE`
/// (≈1.18e-38) — the eDSL body picks the GPU SHAPE (the byte-identity target), exactly as
/// `dist_to_brick_exit_body` picks the GPU's `1.0e30` over the host's `f32::INFINITY`.
pub const M2_REGULA_DENOM_EPS: f32 = 1.0e-30;

/// The byte → normalized-float step of the snorm decode: `q == i8::MIN ? -1 : q/127`.
///
/// On the GPU this is done by the FIXED-FUNCTION `R8_SNORM` sampler (hardware), so the
/// `Emit` instantiation is NEVER spliced into a shader — only the `f32` Eval
/// instantiation runs (the CPU oracle). It byte-mirrors the host
/// `boyko_sdf_math::brick::decode_snorm8`'s normalize (brick.rs:1052):
///
/// ```text
/// let n = if q == i8::MIN { -1.0 } else { q as f32 / 127.0 };
/// ```
///
/// `q` is lifted into the backend integer ([`FieldScalar::Int`]); the sentinel test is
/// a traced [`FieldScalar::int_eq`], the `q/127` arm is a [`FieldScalar::int_to_float`]
/// numeric cast over a [`FieldScalar::div`]. Both arms are pure (no data-dependent
/// control flow), selected by [`FieldScalar::select`] — the same value-select shape the
/// field's `combine` uses.
#[inline]
pub fn snorm_normalize<S: FieldScalar>(q: S::Int) -> S {
    let is_sentinel = S::int_eq(q, S::int_lit(SNORM_SENTINEL));
    // The `q / 127` arm: numeric cast then divide by the snorm divisor.
    let scaled = S::int_to_float(q).div(S::lit(SNORM_DIVISOR));
    // `q == i8::MIN ? -1.0 : q/127` — the asymmetric snorm clamp.
    S::select(is_sentinel, S::lit(-1.0), scaled)
}

/// The world-scale step of the snorm decode: `n * band_half`.
///
/// This IS the GPU `m2_decode(n, band_half)` — a pure `float` op. Its `Emit`
/// instantiation is the body spliced into `sdf_gbuffer_composite.hlsl`'s decode
/// (between the `// === GENERATED decode_snorm8 BEGIN/END ===` sentinels), and its
/// `f32` Eval instantiation is the host's post-normalize multiply (brick.rs:1053).
#[inline]
pub fn snorm_scale<S: FieldScalar>(n: S, band_half: S) -> S {
    n.mul(band_half)
}

/// Decodes one `R8_SNORM` narrow-band code `q` back to a world distance, given the
/// band half-width. The full leaf: `snorm_scale(snorm_normalize(q), band_half)`.
///
/// The `f32` Eval instantiation is BYTE-IDENTICAL to the host
/// `boyko_sdf_math::brick::decode_snorm8` (the CPU oracle the GPU brick fetch is
/// golden-compared against). On the GPU the two steps are split — the hardware sampler
/// does [`snorm_normalize`], the shader's spliced `m2_decode` does [`snorm_scale`] —
/// so only `snorm_scale`'s `Emit` body reaches a shader; see the module doc.
#[inline]
pub fn decode_snorm8<S: FieldScalar>(q: S::Int, band_half: S) -> S {
    snorm_scale(snorm_normalize::<S>(q), band_half)
}

// ---- The M2 cubic-surface leaves (A3) -----------------------------------------
//
// These two are PURE EXPRESSIONS (no data-dependent control flow), so they
// single-source the SAME way the field/decode leaves do: authored ONCE generic
// over `FieldScalar`, the `f32` Eval instantiation is byte-identical to the host
// `boyko_sdf_math::brick::{cubic_eval, jcgt_cubic_coeffs}`, and the `Emit`
// instantiation is spliced into `sdf_gbuffer_composite.hlsl` (`m2_cubic_eval` /
// `m2_jcgt_cubic_coeffs`).
//
// They are the LEAVES of the JCGT cubic solver, not the solver: the
// root-finders (`marmitt_root` / `regula_falsi`) have data-dependent runtime
// control flow (`if disc > 0`, mutable brackets, an iteration loop, early
// returns) and stay HAND-WRITTEN in HLSL + host; those CALL `m2_cubic_eval` /
// `m2_jcgt_cubic_coeffs` exactly as `sdf_normal` calls the hand-written `sdf`.

/// Evaluates the JCGT cubic `c3·t³ + c2·t² + c1·t + c0` at `t` (Horner, FMA-friendly).
///
/// Authored ONCE generic over [`FieldScalar`], byte-mirroring the host
/// `boyko_sdf_math::brick::cubic_eval` (brick.rs:1411) and the GPU `m2_cubic_eval`
/// (`sdf_gbuffer_composite.hlsl:723`) operand-for-operand. The coefficients are
/// passed by VALUE in the `[c0, c1, c2, c3]` order (so `c[3]` is the cubic term);
/// the GPU packs them in a `float4 c` and spells the SAME order as `c.x..c.w`.
///
/// Horner: `((c3·t + c2)·t + c1)·t + c0` — the exact left-to-right grouping the
/// host writes and the GPU transcribes. A reordered FMA chain would drift `t` past
/// the hit/miss cliff, so this MUST NOT be "simplified".
#[inline]
pub fn cubic_eval<S: FieldScalar>(c: &[S; 4], t: S) -> S {
    // ((c3*t + c2)*t + c1)*t + c0 — one multiply-add per term.
    c[3]
        .mul(t)
        .add(c[2])
        .mul(t)
        .add(c[1])
        .mul(t)
        .add(c[0])
}

/// Forms the JCGT-2022 cubic `[c0, c1, c2, c3]` whose root is the
/// ray↔trilinear-isosurface crossing in ONE voxel cell.
///
/// Authored ONCE generic over [`FieldScalar`], byte-mirroring the host
/// `boyko_sdf_math::brick::jcgt_cubic_coeffs` (brick.rs:1353) and the GPU
/// `m2_jcgt_cubic_coeffs` (`sdf_gbuffer_composite.hlsl:732`) operand-for-operand.
///
/// `s` holds the 8 corner distances in the `s_ijk ↔ x + 2·y + 4·z` convention
/// (x fastest): `s[0] = s000`, `s[1] = s100`, ..., `s[7] = s111`. `a` = `ro_local`,
/// `b` = `rd_local` in the cell's `[0,1]³` frame. The 8 corners fold into the
/// trilinear k-basis, then the ray `(x,y,z) = a + b·t` is substituted and powers of
/// `t` collected. The index pairing (the k3/k7 trap) and the FMA grouping are
/// load-bearing: a transposed pair or a reordered expansion silently samples the
/// wrong interpolant / drifts the golden — this MUST NOT be "simplified".
///
/// The `Emit` instantiation returns through this `[S; 4]` array; the printer
/// (`crate::emit::emit_hlsl_jcgt_cubic_coeffs`) emits the array as the GPU's
/// `float4(c0, c1, c2, c3)` construct.
#[inline]
pub fn jcgt_cubic_coeffs<S: FieldScalar>(s: &[S; 8], a: [S; 3], b: [S; 3]) -> [S; 4] {
    // Fold the 8 corners into the trilinear k-basis (the `x + 2y + 4z` corner order).
    let s000 = s[0];
    let s100 = s[1];
    let s010 = s[2];
    let s110 = s[3];
    let s001 = s[4];
    let s101 = s[5];
    let s011 = s[6];
    let s111 = s[7];

    let k0 = s000;
    let k1 = s100.sub(s000);
    let k2 = s010.sub(s000);
    let k3 = s001.sub(s000);
    let k4 = s110.sub(s100).sub(s010).add(s000); // x·y
    let k5 = s011.sub(s010).sub(s001).add(s000); // y·z
    let k6 = s101.sub(s100).sub(s001).add(s000); // z·x
    // x·y·z
    let k7 = s111
        .sub(s110)
        .sub(s101)
        .sub(s011)
        .add(s100)
        .add(s010)
        .add(s001)
        .sub(s000);

    let (ax, ay, az) = (a[0], a[1], a[2]);
    let (bx, by, bz) = (b[0], b[1], b[2]);

    // c0 = k0 + k1*ax + k2*ay + k3*az + k4*ax*ay + k5*ay*az + k6*az*ax + k7*ax*ay*az
    let c0 = k0
        .add(k1.mul(ax))
        .add(k2.mul(ay))
        .add(k3.mul(az))
        .add(k4.mul(ax).mul(ay))
        .add(k5.mul(ay).mul(az))
        .add(k6.mul(az).mul(ax))
        .add(k7.mul(ax).mul(ay).mul(az));

    // c1 = k1*bx + k2*by + k3*bz
    //    + k4*(ax*by + ay*bx) + k5*(ay*bz + az*by) + k6*(az*bx + ax*bz)
    //    + k7*(ax*ay*bz + ax*by*az + bx*ay*az)
    let c1 = k1
        .mul(bx)
        .add(k2.mul(by))
        .add(k3.mul(bz))
        .add(k4.mul(ax.mul(by).add(ay.mul(bx))))
        .add(k5.mul(ay.mul(bz).add(az.mul(by))))
        .add(k6.mul(az.mul(bx).add(ax.mul(bz))))
        .add(k7.mul(
            ax.mul(ay).mul(bz).add(ax.mul(by).mul(az)).add(bx.mul(ay).mul(az)),
        ));

    // c2 = k4*bx*by + k5*by*bz + k6*bz*bx + k7*(ax*by*bz + bx*ay*bz + bx*by*az)
    let c2 = k4
        .mul(bx)
        .mul(by)
        .add(k5.mul(by).mul(bz))
        .add(k6.mul(bz).mul(bx))
        .add(k7.mul(
            ax.mul(by).mul(bz).add(bx.mul(ay).mul(bz)).add(bx.mul(by).mul(az)),
        ));

    // c3 = k7*bx*by*bz
    let c3 = k7.mul(bx).mul(by).mul(bz);

    [c0, c1, c2, c3]
}

// ---- The brick-exit empty-skip marcher leaf (Increment 1: control flow) ---------
//
// The FIRST leaf with CONTROL FLOW: an `[unroll]` `for a in 0..3` slab loop with a
// data-dependent `continue` (the near-axis-parallel skip) and a final `Select`
// (the progress clamp). Authored ONCE generic over the control-flow axis `C: Cf`,
// whose `C::Scalar` fixes the value arithmetic per backend. Instantiated:
//   - `<EvalCf>` (`Scalar = f32`)  — the CPU oracle (real `for`/`if`/`continue`); the
//     `eval_byte_identity` brick-exit sweep locks it.
//   - `<EmitCf>` (`Scalar = Emit`) — the HLSL recorder; the printer
//     (`crate::emit::emit_hlsl_dist_to_brick_exit`) walks the STMT IR into the
//     `[unroll]`/`for`/`continue` body spliced into `sdf_gbuffer_composite.hlsl`.
//
// CANONICAL FORM = the GPU SHAPE (the committed `dist_to_brick_exit`,
// sdf_gbuffer_composite.hlsl:569-589): `exit` inits to a plain `1.0e30` literal,
// `.max()`/`.min()` for the per-axis far-face / running exit, NO `is_finite` term.
// The HOST `boyko_sdf_math::brick::dist_to_brick_exit` (which inits `f32::INFINITY`
// and has a final `|| !exit.is_finite()` guard) STAYS HAND-WRITTEN and does NOT
// delegate (firewall option B) — they already diverge on an all-axes-degenerate ray,
// which is marcher-UNREACHABLE (a normalized `rd` cannot have all three |components|
// <= 1e-4). This body picks the GPU shape; its single-source authority is the EvalCf
// to-bits sweep, not a host call.

/// The ray-AABB SLAB exit distance for the brick at `cell_min` of size `bw`, from `p`
/// along `rd` — the empty-skip step length, authored ONCE over the value axis `S` and
/// the control-flow axis `C`.
///
/// Returns the `t` at which the ray leaves the brick's `[cell_min, cell_min + bw]`
/// AABB, measured from `p`. Standard slab method: per axis the far-face crossing is
/// `max(t_lo, t_hi)`, and the brick exit is the `min` over the three axes; a near-axis-
/// parallel component (`abs(dir) <= BRICK_EXIT_EPS`) is SKIPPED (`continue`). The final
/// `exit < BRICK_EXIT_EPS ? BRICK_EXIT_EPS : exit` is the PROGRESS GUARANTEE (the march
/// must always advance — INVIOLABLE for the empty skip).
///
/// This is the GPU-SHAPE canonical form (see the module comment): `exit` inits to a
/// plain `1.0e30` and there is no `is_finite` term. The eval sweep proves the dropped
/// `is_finite` changes no output bit on the reachable (normalized-ray) set.
#[inline]
pub fn dist_to_brick_exit_body<C: Cf>(
    p: [C::Scalar; 3],
    rd: [C::Scalar; 3],
    cell_min: [C::Scalar; 3],
    bw: C::Scalar,
) -> C::Scalar {
    // The value type IS `C::Scalar` (`f32` on Eval, `Emit` on Emit); bound locally for
    // readability so the body reads `S::lit(..)` / `S::select(..)` as before.
    type S2<C> = <C as Cf>::Scalar;
    // `float exit = 1.0e30;` — the GPU init (a plain literal, NOT f32::INFINITY).
    let exit = C::decl_var("exit", S2::<C>::lit(1.0e30));
    C::unroll_for("[unroll]", 3, |a| -> Flow {
        // The MATERIALIZED slab temps, in program order (each `C::temp` becomes a
        // `float tN = ...;`; on Eval `temp` is identity). The materialization choice
        // (which subexpressions are `tN` locals) is pinned to the committed HLSL so the
        // generator re-DXCs byte-identical — see `Cf::temp`. `t0 = rd[a]`, `t1 =
        // cell_min[a]` and `t2 = t1 + bw` are computed BEFORE the skip guard (the original
        // author's order); `abs(t0)` and `p[a]` stay INLINE (un-`temp`'d).
        let dir = C::temp(C::index(rd, a)); // float t0 = rd[a];
        let lo = C::temp(C::index(cell_min, a)); // float t1 = cell_min[a];
        let hi = C::temp(lo.add(bw)); // float t2 = t1 + bw;
        // if (abs(t0) <= BRICK_EXIT_EPS) { continue; } — the near-axis-parallel skip.
        // `?` propagates the continue token out of this closure (the live tail below is
        // skipped), which `unroll_for` maps to a real `continue` (Eval) / `Stmt::Continue`
        // (Emit). `<=` is a DISTINCT opcode (OpFOrdLessThanEqual) from a swapped `>`.
        let eps = C::named_lit("BRICK_EXIT_EPS", BRICK_EXIT_EPS);
        C::if_(dir.abs().le(eps), C::cont)?;
        // float t3 = 1.0/t0; float t4 = (t1 - p[a]) * t3; float t5 = (t2 - p[a]) * t3;
        // float t6 = max(t4, t5);  exit = min(exit, t6);  (`p[a]` stays inline.)
        let inv = C::temp(S2::<C>::lit(1.0).div(dir)); // float t3 = 1.0 / t0;
        let t_lo = C::temp(lo.sub(C::index(p, a)).mul(inv)); // float t4 = (t1 - p[a]) * t3;
        let t_hi = C::temp(hi.sub(C::index(p, a)).mul(inv)); // float t5 = (t2 - p[a]) * t3;
        let t_far = C::temp(t_lo.max(t_hi)); // float t6 = max(t4, t5);
        C::set_var(&exit, C::get_var(&exit).min(t_far)); // exit = min(exit, t6);
        Flow::Continue(())
    });
    // return (exit < BRICK_EXIT_EPS) ? BRICK_EXIT_EPS : exit;  (the ternary stays inline.)
    let final_exit = C::get_var(&exit);
    let eps = C::named_lit("BRICK_EXIT_EPS", BRICK_EXIT_EPS);
    S2::<C>::select(final_exit.lt(eps), eps, final_exit)
}

// ---- The brick-cell pointer-grid lookup leaf (Increment 3: early-return CF) ------
//
// The SECOND control-flow leaf — the first with EARLY RETURNS, a `StructuredBuffer<uint>`
// load, an `out float3` parameter, and `uint` index math (a `float3`/`uint` value model).
// Authored ONCE generic over the control-flow axis `C: Cf`; `C::Scalar` fixes the float
// arithmetic, the typed facets (`C::Uint`/`C::Vec3f`/`C::Uint3`/`C::Buf`/`C::OutVec3`)
// fix the brick value model. Instantiated:
//   - `<EvalCf>` — the CPU oracle (real casts / real `||` / a `Cell` out-param + ret-cell);
//     the `eval_byte_identity` brick-cell sweep + a tail-skip test lock it.
//   - `<EmitCf>` — the HLSL recorder; the printer
//     (`crate::emit::emit_hlsl_brick_cell_class`) walks the STMT IR into the early-return
//     body spliced into `sdf_gbuffer_composite.hlsl` (byte-identical to the committed
//     `.comp.spv`, proven by the one-shot cmp-`.spv`).
//
// RET is the SOLE return mechanism: each guard records EXACTLY ONE `Stmt::Return`; the
// VALUE travels OUT OF BAND (a body-local cell on Eval; the value node into `Stmt::Return`
// on Emit). The body NEVER writes a `__cls`/`__ret` local — no spurious assign reaches the
// emitted HLSL. The out-param `cell_min` is written by TWO `out_vec3_assign`s (the default
// `cell_min = origin;` and the conditional `cell_min = origin + float3(...)*bw;`), each a
// bare assignment (no `float3` decl).

/// Reads the pointer-grid cell class containing world point `p`, depositing the class into
/// `cls` (`BRICK_OUTSIDE_GRID` when `p` is outside the bounded grid) and the cell's world
/// minimum corner into the `cell_min` out-param. Authored ONCE over the control-flow axis
/// `C` + the brick value model. Mirrors the GPU `brick_cell_class`
/// (`sdf_gbuffer_composite.hlsl:608-626`) statement-for-statement.
///
/// The class travels OUT OF BAND through `cls` (the [`Cf::RetCell`]) — on Eval the caller
/// reads `cls` after this returns; on Emit the recorded `Stmt::Return`s spell `return
/// <expr>;`. The body order matches the committed HLSL: the default `cell_min`, guard 1
/// (negative-rel), the `(uint)` casts, guard 2 (bounds), the `idx`, the conditional
/// `cell_min`, and the buffer-load return.
///
/// The body is an IIFE `(|| -> Flow { ... })()` whose `?`-propagated [`Cf::ret`] /
/// [`Cf::if_ret`] short-circuit the closure on Eval (so the tail — the casts after guard 1
/// — does NOT run on a negative rel, the load-bearing tail-skip), then the caller reads
/// `cls`. On Emit every branch is recorded structurally (the `?` never early-returns), so
/// the whole body is captured.
#[inline]
pub fn brick_cell_class_body<C: Cf>(
    grid: C::Buf<'_>,
    origin: C::Vec3f,
    bw: C::Scalar,
    dims: C::Uint3,
    p: C::Vec3f,
    cell_min: &C::OutVec3,
    cls: &C::RetCell,
) {
    // The IIFE: the guard logic runs in a closure returning `Flow`. On Eval the first
    // `?`-propagated `ret`/`if_ret` short-circuits the closure (the deposited class is read
    // from `cls` afterward); on Emit every statement is recorded (the closure runs to the
    // end, returning `Continue`). The `Flow` result is discarded either way.
    let run = || -> Flow {
        // float3 rel = (p - origin) / bw;
        let rel = C::temp_vec3("rel", C::vec3_div_scalar(C::vec3_sub(p, origin), bw));
        // cell_min = origin;  (default — overwritten on an in-grid hit; unread when OUTSIDE)
        C::out_vec3_assign(cell_min, origin);
        // if (rel.x < 0.0 || rel.y < 0.0 || rel.z < 0.0) { return BRICK_OUTSIDE_GRID; }
        // The float `<` is tested directly (a negative coord is caught BEFORE the uint cast
        // wraps it). The `||` is lazy on Emit (short-circuit OpBranchConditional, spike E2a)
        // and result-equivalent on Eval (the comparands are pure); the tail-skip (the casts
        // below not running on a negative rel) is the `?` early-returning the IIFE.
        let zero = C::Scalar::lit(0.0);
        let neg = C::or(
            C::or(C::vec3_x(rel).lt(zero), C::vec3_y(rel).lt(zero)),
            C::vec3_z(rel).lt(zero),
        );
        C::if_ret(cls, neg, C::named_uint("BRICK_OUTSIDE_GRID", BRICK_OUTSIDE_GRID))?;
        // uint ix = (uint)rel.x;  uint iy = (uint)rel.y;  uint iz = (uint)rel.z;
        let ix = C::temp_uint("ix", C::float_to_uint(C::vec3_x(rel)));
        let iy = C::temp_uint("iy", C::float_to_uint(C::vec3_y(rel)));
        let iz = C::temp_uint("iz", C::float_to_uint(C::vec3_z(rel)));
        // if (ix >= dims.x || iy >= dims.y || iz >= dims.z) { return BRICK_OUTSIDE_GRID; }
        let oob = C::or(
            C::or(C::uge(ix, C::uint3_x(dims)), C::uge(iy, C::uint3_y(dims))),
            C::uge(iz, C::uint3_z(dims)),
        );
        C::if_ret(cls, oob, C::named_uint("BRICK_OUTSIDE_GRID", BRICK_OUTSIDE_GRID))?;
        // uint idx = ix + iy * dims.x + iz * dims.x * dims.y;
        // Flat left-associated: ((ix + (iy*dims.x)) + ((iz*dims.x)*dims.y)). The
        // position-aware paren keeps the emitted text flat (`ix + iy * dims.x + ...`).
        let idx = C::temp_uint(
            "idx",
            C::uadd(
                C::uadd(ix, C::umul(iy, C::uint3_x(dims))),
                C::umul(C::umul(iz, C::uint3_x(dims)), C::uint3_y(dims)),
            ),
        );
        // cell_min = origin + float3(ix, iy, iz) * bw;
        C::out_vec3_assign(
            cell_min,
            C::vec3_add(
                origin,
                C::vec3_mul_scalar(C::vec3_from_uints(ix, iy, iz), bw),
            ),
        );
        // return grid[idx];
        C::ret(cls, C::buffer_load(grid, idx))?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the early `?` already deposited the class into `cls`; on
    // Emit the recorder captured every statement.
    let _ = run();
}

// ---- The regula-falsi root refinement leaf (Increment 4a: a RUNTIME `[loop]`) ----
//
// The THIRD control-flow leaf — the FIRST with a genuine RUNTIME loop (an `OpLoop`, vs
// `dist_to_brick_exit`'s `[unroll]`). `m2_regula_falsi` carries FIVE Phi vars across a
// const-bounded `[loop]` (whose header spells the BOUND SYMBOL `M2_MARMITT_ITERS`, not a
// `<n>u` literal): {lo, hi, f_lo, f_hi} as SIGNATURE PARAMS (suppressed-decl) + `mid` as a
// TRUE local. It has an in-loop EARLY RETURN (`return mid;`), forwarded through the runtime
// loop to the function-scope IIFE by `?` (so the early `mid` — not the final-iteration
// `mid` — is returned). Authored ONCE over the control-flow axis `C: Cf` + a field-call
// seam (the frozen `m2_cubic_eval`). Instantiated:
//   - `<EvalCf>` — the CPU oracle (real `for`/`if`-`else`/`Cell` + the host cubic closure);
//     the `eval_byte_identity` regula-falsi sweep (incl. the early-return-at-k<8 case) locks
//     it against a frozen GPU-shape reference.
//   - `<EmitCf>` — the HLSL recorder; the printer
//     (`crate::emit::emit_hlsl_m2_regula_falsi`) walks the STMT IR into the `[loop]` body
//     spliced into `sdf_gbuffer_composite.hlsl` (byte-identical to the committed
//     `.comp.spv`, proven by the cmp-`.spv`).
//
// GPU-SHAPE CANONICAL FORM: the degenerate-bracket guard uses `1.0e-30`
// ([`M2_REGULA_DENOM_EPS`]), the COMMITTED GPU literal — NOT the host `regula_falsi`'s
// `f32::MIN_POSITIVE`. The two ALREADY diverge on a near-flat secant whose `|denom|` lands
// in `(1.18e-38, 1.0e-30)` (the GPU bisects, the host secant-steps), so the host
// `regula_falsi` is NOT the oracle: the eval sweep pins this body against a frozen GPU-shape
// reference (the same single-source discipline `dist_to_brick_exit_body` uses).

/// Refines a sign-bracketed root of the JCGT cubic in `[lo, hi]` by regula-falsi (false
/// position), depositing the refined `mid` into `out`. Authored ONCE over the control-flow
/// axis `C` + the cubic-eval seam `eval`. Mirrors the GPU `m2_regula_falsi`
/// (`sdf_gbuffer_composite.hlsl:867-885`) statement-for-statement.
///
/// `c` is the cubic coefficients (call-through-only — passed to `eval`, never swizzled);
/// `lo`/`hi`/`f_lo`/`f_hi` are the bracket ends and their function values. `eval` is the
/// cubic-eval seam (see [`crate::normal`]'s field-call seam): on Eval it is the host
/// `cubic_eval` closure (so `m2_regula_falsi_body::<EvalCf>` re-runs the host cubic at each
/// `mid`); on Emit it records a `m2_cubic_eval(c, mid)` call node (via [`Cf::call2`]).
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; ret_f(out, mid)?; Continue }`,
/// so an in-loop [`Cf::if_ret_f`]'s `Break(Return)` forwards through [`Cf::runtime_for`]'s
/// `?` to the IIFE's `?` — skipping the tail `ret_f` (the early `mid` is the result, NOT the
/// 8-iteration `mid`). On Emit every branch is recorded structurally (`?` never early-
/// returns); the whole body is captured.
#[inline]
pub fn m2_regula_falsi_body<C: Cf, F: Fn(C::Vec4f, C::Scalar) -> C::Scalar>(
    c: C::Vec4f,
    lo0: C::Scalar,
    hi0: C::Scalar,
    f_lo0: C::Scalar,
    f_hi0: C::Scalar,
    eval: F,
    out: &C::RetCellF,
) {
    let run = || -> Flow {
        // The four carried params are SIGNATURE parameters (suppressed-decl — get/set spell
        // `lo`/`hi`/..., but NO `float lo = ...;` redecl is recorded).
        let lo = C::decl_param("lo", lo0);
        let hi = C::decl_param("hi", hi0);
        let f_lo = C::decl_param("f_lo", f_lo0);
        let f_hi = C::decl_param("f_hi", f_hi0);
        // `float mid = lo;` — a TRUE local (a recorded DeclVar).
        let mid = C::decl_var("mid", C::get_var(&lo));

        C::runtime_for(
            "[loop]",
            "i",
            "M2_MARMITT_ITERS",
            M2_MARMITT_ITERS,
            |_i| -> Flow {
                // float denom = f_hi - f_lo;
                let denom = C::temp_float("denom", C::get_var(&f_hi).sub(C::get_var(&f_lo)));
                // mid = (abs(denom) > 1.0e-30) ? (lo - f_lo * (hi - lo) / denom) : (0.5 * (lo + hi));
                let denom_eps = C::Scalar::lit(M2_REGULA_DENOM_EPS);
                // The secant step `lo - f_lo * (hi - lo) / denom` (left-to-right: `(hi - lo)`
                // then `/ denom` then `* f_lo` then `lo - ...`).
                let secant = C::get_var(&lo).sub(
                    C::get_var(&f_lo)
                        .mul(C::get_var(&hi).sub(C::get_var(&lo)))
                        .div(denom),
                );
                // The bisection fallback `0.5 * (lo + hi)`.
                let bisect = C::Scalar::lit(0.5).mul(C::get_var(&lo).add(C::get_var(&hi)));
                C::set_var(&mid, C::select(denom.abs().gt(denom_eps), secant, bisect));
                // float f_mid = m2_cubic_eval(c, mid);
                let f_mid = C::temp_float("f_mid", eval(c, C::get_var(&mid)));
                // if (abs(f_mid) <= M2_CUBIC_ROOT_EPS || (hi - lo) <= M2_CUBIC_ROOT_EPS) { return mid; }
                let root_eps = C::named_lit("M2_CUBIC_ROOT_EPS", M2_CUBIC_ROOT_EPS);
                let converged = C::or(
                    f_mid.abs().le(root_eps),
                    C::get_var(&hi).sub(C::get_var(&lo)).le(root_eps),
                );
                // The `?` forwards a Break(Return) through runtime_for to the function IIFE.
                C::if_ret_f(out, converged, C::get_var(&mid))?;
                // if (f_lo * f_mid <= 0.0) { hi = mid; f_hi = f_mid; } else { lo = mid; f_lo = f_mid; }
                let bracket = C::get_var(&f_lo).mul(f_mid).le(C::Scalar::lit(0.0));
                // The bracket update is a two-arm branch with no continue/return (both arms
                // fall through), so its `Flow` is always `Continue` — discard it (no `?`).
                let _ = C::if_else(
                    bracket,
                    || {
                        C::set_var(&hi, C::get_var(&mid));
                        C::set_var(&f_hi, f_mid);
                        Flow::Continue(())
                    },
                    || {
                        C::set_var(&lo, C::get_var(&mid));
                        C::set_var(&f_lo, f_mid);
                        Flow::Continue(())
                    },
                );
                Flow::Continue(())
            },
        )?;
        // return mid;  (reached only when the loop completes its budget without converging.)
        C::ret_f(out, C::get_var(&mid))?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the early `?` already deposited the refined `mid` into `out`;
    // on Emit the recorder captured every statement.
    let _ = run();
}

// ---- The brick-AABB ray-span clip leaf (Increment 5b: a COMPUTED-bool return) ----
//
// The FOURTH-AND-A-HALF control-flow leaf (the second `[unroll]` body after
// `dist_to_brick_exit`) — `m2_brick_span` clips the ray `p + rd·t` to ONE brick's
// `[cell_min, cell_min + brick_world]` AABB by the standard slab method, returning the
// `[t_enter, t_exit]` span through TWO `out float` params + a COMPUTED `bool` (`tmax >
// tmin` — a hit). It reuses the Inc-5a Cf-axis machinery (the `runtime_for` loop, the two
// `OutFloat` params, the swap `if_`) + adds three small facets:
//   - the COMPUTED-bool return (`Cf::ret_b_expr` — `return tmax > tmin;`, a Mask operand,
//     vs the `bool`-LITERAL `Cf::ret_b`'s `return false;`);
//   - TWO `out float` params (`t_enter`/`t_exit`), each written by `Cf::out_float_assign`;
//   - a FALL-THROUGH `if_` (the swap `if (t1 > t2) { float tmp = t1; t1 = t2; t2 = tmp; }`)
//     whose body runs 3 statements then FALLS THROUGH (returns `Flow::Continue(())`) — the
//     same fall-through-body shape `m2_regula_falsi`'s bracket-update `if_else` already
//     proved (`if_` accepts a `FnOnce() -> Flow`; a `Continue` body records a `Stmt::If`
//     then falls through on Emit, runs the block then continues on Eval).
//
// THE LOOP IS `runtime_for`, NOT `unroll_for`: the early `return false` inside the loop is a
// `Break(LoopOp::Return)` that must FORWARD through the loop to the function-scope IIFE's
// `?` — which `runtime_for` does (its `Break(Return)` arm forwards), but `unroll_for` does
// NOT (its `Return` arm is a `debug_assert!(false)` — an unrolled loop is assumed to have no
// early return, as `dist_to_brick_exit` does not). `runtime_for("[unroll]", "a", "3u", 3,
// ...)` emits `[unroll]\nfor (uint a = 0u; a < 3u; ++a)` — BYTE-IDENTICAL text to the
// committed `[unroll] for (... a < 3u ...)` header (DXC sees only text, so the re-DXC'd
// SPIR-V is unchanged), while correctly forwarding the in-loop return.
//
// CANONICAL FORM = the GPU SHAPE (the committed `m2_brick_span`,
// sdf_gbuffer_composite.hlsl:969-994): `tmax` inits to a plain `1.0e30` literal, the
// parallel-slab guard is `abs(rd[a]) <= 1.0e-20`, and the literals `1.0`/`0.0`/`1.0e30`/
// `1.0e-20` spell DIRECTLY (not symbols) — `fmt_lit`'s shortest-round-trip compiles to the
// SAME `OpConstant`, gated by the cmp-`.spv`. There is no host `m2_brick_span` mirror in the
// engine; the eval oracle is a verbatim host transcription of the committed span
// (`eval_byte_identity`'s `host_m2_brick_span`), the SAME GPU-shape single-source discipline
// `dist_to_brick_exit_body` uses.

/// `float tmin = 0.0;` — the entry-side clamp (never march behind the current point).
const SPAN_TMIN_INIT: f32 = 0.0;

/// `float tmax = 1.0e30;` — the far-side init (the GPU shape's plain literal). DXC-fold-neutral
/// (`fmt_lit` round-trips to the same `OpConstant`).
const SPAN_TMAX_INIT: f32 = 1.0e30;

/// The near-axis-parallel guard `abs(rd[a]) <= 1.0e-20` — a slab whose direction component is
/// (near-)zero is parallel, so it contributes no near/far crossing; the origin must lie inside it
/// or the whole brick is missed. Mirrors the committed literal (NOT a symbol).
const SPAN_PARALLEL_EPS: f32 = 1.0e-20;

/// The empty-span `t_enter` sentinel (`t_enter = 1.0;`) written on a parallel-slab miss — paired
/// with `SPAN_EMPTY_EXIT` so `t_exit < t_enter` flags the miss to the caller.
const SPAN_EMPTY_ENTER: f32 = 1.0;

/// The empty-span `t_exit` sentinel (`t_exit = 0.0;`) — see `SPAN_EMPTY_ENTER`.
const SPAN_EMPTY_EXIT: f32 = 0.0;

/// Clips the ray `p + rd·t` to the M2 brick at `cell_min` of size `brick_world`, depositing the
/// `[t_enter, t_exit]` world-`t` span (measured from `p`) into the two `out float` params and
/// returning a COMPUTED `bool` (`tmax > tmin` — true on a real intersection) into `ret_out`.
/// Authored ONCE over the control-flow axis `C`. Mirrors the GPU `m2_brick_span`
/// (`sdf_gbuffer_composite.hlsl:969-994`) statement-for-statement (the hand-written signature +
/// closing brace stay un-generated, framing (b)).
///
/// `p`/`rd` are the world ray origin/direction; `cell_min` the brick's lower world corner;
/// `brick_world` the brick edge length. The span travels OUT OF BAND through `t_enter`/`t_exit`
/// (the two [`Cf::OutFloat`]s) and the hit through `ret_out` (the [`Cf::RetCellB`]) — on Eval the
/// caller reads them after this returns; on Emit the recorded `Stmt::OutAssign`s / `Stmt::Return`
/// spell the writes.
///
/// The body is a FUNCTION-SCOPE IIFE `run = || -> Flow { ...; ret_b_expr(ret_out, tmax > tmin)?;
/// Continue }`, so the in-loop early `return false` (a parallel-slab miss — [`Cf::ret_b`] inside the
/// inner `if_`) forwards through [`Cf::runtime_for`]'s `?` to the IIFE's `?`, skipping the tail (the
/// empty `t_enter = 1.0; t_exit = 0.0;` is written FIRST, so the caller reads them). The
/// parallel-slab pass-through is a [`Cf::cont`] (consumed by `runtime_for`). On Emit every branch is
/// recorded structurally (`?` never early-returns); the whole body is captured.
#[inline]
pub fn m2_brick_span_body<C: Cf>(
    p: [C::Scalar; 3],
    rd: [C::Scalar; 3],
    cell_min: [C::Scalar; 3],
    brick_world: C::Scalar,
    t_enter: &C::OutFloat,
    t_exit: &C::OutFloat,
    ret_out: &C::RetCellB,
) {
    // The `p`/`rd`/`cell_min` `float3` PARAMETERS are indexed per-axis by the unroll iv (`p[a]`),
    // so they thread as `[C::Scalar; 3]` (the `dist_to_brick_exit` index discipline — three copies
    // of one `VecParamRef` on Emit, a real array on Eval), NOT the first-class `C::Vec3f` (the body
    // never uses them as a whole `float3`).
    type S<C> = <C as Cf>::Scalar;

    let run = || -> Flow {
        // float tmin = 0.0;  float tmax = 1.0e30;  — the two MUTABLE carried locals (`tmin =
        // max(tmin, t1)` / `tmax = min(tmax, t2)` are set-form updates, so they are `decl_var`s, NOT
        // read-only temps).
        let tmin = C::decl_var("tmin", S::<C>::lit(SPAN_TMIN_INIT));
        let tmax = C::decl_var("tmax", S::<C>::lit(SPAN_TMAX_INIT));

        C::runtime_for("[unroll]", "a", "3u", 3, |a| -> Flow {
            // float lo = cell_min[a];  float hi = lo + brick_world;  — read-only NAMED `float`
            // temps (`temp_float` returns the value directly; on Eval identity, on Emit a
            // `float lo = ...;` decl). Never reassigned, so they are temps, NOT `decl_var`s.
            let lo = C::temp_float("lo", C::index(cell_min, a));
            let hi = C::temp_float("hi", lo.add(brick_world));
            // if (abs(rd[a]) <= 1.0e-20) { ... continue; }  — the parallel-slab branch. Its body is
            // an INNER early-return `if_` (the outside-the-slab miss) THEN a `continue` (the
            // pass-through). The outer `if_`'s `?` forwards whichever token the body yields
            // (`Break(Return)` from the inner miss, or `Break(Continue)` from `cont`).
            let eps = S::<C>::lit(SPAN_PARALLEL_EPS);
            C::if_(C::index(rd, a).abs().le(eps), || -> Flow {
                // if (p[a] < lo || p[a] > hi) { t_enter = 1.0; t_exit = 0.0; return false; }
                // The origin is outside this parallel slab → the whole brick is missed. The two
                // empty-span out-writes happen BEFORE the `ret_b` short-circuits, so the caller
                // reads the `1.0`/`0.0` sentinels (`t_exit < t_enter` flags the miss).
                let outside = C::or(C::index(p, a).lt(lo), C::index(p, a).gt(hi));
                C::if_(outside, || -> Flow {
                    C::out_float_assign(t_enter, S::<C>::lit(SPAN_EMPTY_ENTER));
                    C::out_float_assign(t_exit, S::<C>::lit(SPAN_EMPTY_EXIT));
                    C::ret_b(ret_out, false)
                })?;
                // continue;  — the slab is parallel but the origin is inside it, so it imposes no
                // near/far bound; skip the rest of this axis.
                C::cont()
            })?;
            // float inv = 1.0 / rd[a];  — read-only NAMED temp.
            let inv = C::temp_float("inv", S::<C>::lit(1.0).div(C::index(rd, a)));
            // float t1 = (lo - p[a]) * inv;  float t2 = (hi - p[a]) * inv;  — the two slab crossings,
            // MUTABLE (swapped below) → `decl_var`s.
            let t1 = C::decl_var("t1", lo.sub(C::index(p, a)).mul(inv));
            let t2 = C::decl_var("t2", hi.sub(C::index(p, a)).mul(inv));
            // if (t1 > t2) { float tmp = t1; t1 = t2; t2 = tmp; }  — order the crossings so t1 is
            // near, t2 far. A FALL-THROUGH `if_` (no continue/break/return; runs 3 statements then
            // returns `Flow::Continue(())`). `tmp` captures t1's OLD value BEFORE t1 is overwritten
            // (`temp_float` reads `get_var(&t1)` eagerly), so the swap is correct on Eval.
            C::if_(C::get_var(&t1).gt(C::get_var(&t2)), || -> Flow {
                let tmp = C::temp_float("tmp", C::get_var(&t1));
                C::set_var(&t1, C::get_var(&t2));
                C::set_var(&t2, tmp);
                Flow::Continue(())
            })?;
            // tmin = max(tmin, t1);  tmax = min(tmax, t2);  — the running near/far span bounds.
            C::set_var(&tmin, C::get_var(&tmin).max(C::get_var(&t1)));
            C::set_var(&tmax, C::get_var(&tmax).min(C::get_var(&t2)));
            Flow::Continue(())
        })?;
        // t_enter = tmin;  t_exit = tmax;  return tmax > tmin;  — the span + the COMPUTED hit (a
        // real intersection has a non-empty `[tmin, tmax]`). `ret_b_expr` records the mask `tmax >
        // tmin` as the return value (vs the `bool`-literal `ret_b`).
        C::out_float_assign(t_enter, C::get_var(&tmin));
        C::out_float_assign(t_exit, C::get_var(&tmax));
        C::ret_b_expr(ret_out, C::get_var(&tmax).gt(C::get_var(&tmin)))?;
        Flow::Continue(())
    };
    // Discard the Flow: on Eval the early `?` already deposited the empty span + `false` into the
    // cells; on Emit the recorder captured every statement.
    let _ = run();
}
