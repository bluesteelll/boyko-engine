//! The brick-atlas `R8_SNORM` decode leaves, authored ONCE generic over
//! [`FieldScalar`] (A2 — the first INTEGER/bit leaf).
//!
//! [`decode_snorm8`] is the inverse of the [`fill_brick`] snorm encode: a stored
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

use crate::scalar::FieldScalar;

/// The `R8_SNORM` normalize divisor — `q ∈ [-127, 127]` maps onto `[-1, 1]` as
/// `q / 127`. Mirrors the host `decode_snorm8`'s `127.0` (brick.rs:1052) and the
/// Vulkan `R8_SNORM` rule.
pub const SNORM_DIVISOR: f32 = 127.0;

/// The snorm sentinel code: `i8::MIN` (-128). The asymmetric `R8_SNORM` rule maps it
/// (and -127) to `-1.0`. Mirrors the host `decode_snorm8`'s `i8::MIN` branch.
pub const SNORM_SENTINEL: i32 = i8::MIN as i32;

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
/// ([`crate::emit::emit_hlsl_jcgt_cubic_coeffs`]) emits the array as the GPU's
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
