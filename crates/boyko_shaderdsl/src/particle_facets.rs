//! The rung-E FACET PROBES — one minimal generic body per new [`Cf`] facet family
//! (docs/PARTICLES-PLAN.md rung E), authored ONCE and instantiated over BOTH backends.
//!
//! Rung E lands the eDSL nodes the seven particle leaves need before any of those leaves can
//! be authored: E1 the bitwise/shift `uint` ops (`particle_rng`'s PCG32), E2 the bit-cast +
//! half-precision conversions (the packed per-particle attributes), E3 the `dot` intrinsic
//! (`particle_sdf_response`), E4 the two transcendentals (the billboard corner spin). A node
//! with no USE SITE cannot be exercised on either backend, so each family gets a probe here:
//! the smallest body that spells the facet the way a real leaf will, instantiated
//!
//! - `<EvalCf>` — the CPU oracle (real integer/float ops, the value deposited in the ret cell);
//! - `<EmitCf>` — the HLSL recorder (`crate::emit::emit_hlsl_e*`, printed by the facet test).
//!
//! # These are PROBES, not shader spans
//!
//! NONE of these bodies is spliced into any shader: there is no
//! `// === GENERATED … BEGIN/END ===` sentinel pair for them and no `.spv` depends on their
//! text, so they cannot fork a committed binary. They exist to give every rung-E node a
//! two-backend use site until the P0 particle leaves land, at which point the leaves become
//! the real consumers and these probes remain as the per-node facet pins (the same role
//! [`crate::decl`]'s one-statement bodies play for the `bool` decl facet).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The PCG-shaped rotation extract — `state >> 28u` yields a 4-bit rotation amount.
const ROT_SHIFT: u32 = 28;
/// The PCG-shaped diffusion shift — `state << 13u`.
const XSH_SHIFT: u32 = 13;
/// The low-16 mask (`0xFFFF`), spelled in decimal like every other `uint` literal the printer
/// emits.
const LOW16_MASK: u32 = 65535;
/// The high-half shift of a packed `float2` half-precision pair.
const HALF2_HI_SHIFT: u32 = 16;
/// The `f32` sign bit as a mask — flipping it negates the value without touching the
/// exponent/significand (the bit-cast round trip the E2 probe pins).
const F32_SIGN_MASK: u32 = 0x8000_0000;

/// **E1** — the bitwise/shift probe: a PCG-shaped fold over all five `uint` bit ops
/// ([`Cf::shr_u`], [`Cf::uxor`], [`Cf::ushl`], [`Cf::and_u`], [`Cf::uor`]), depositing the
/// mixed word into `ret_out`.
///
/// ```text
/// uint rot = state >> 28u;
/// uint word = state ^ (state << 13u);
/// uint tail = word & 65535u;
/// return tail | rot;
/// ```
///
/// The `state ^ (state << 13u)` line is load-bearing beyond the ops themselves: it pins the
/// PRECEDENCE rule, since `<<` binds LOOSER than `^` and an un-parenthesized `state ^ state <<
/// 13u` would parse as `state ^ (state << 13u)` in C but is spelled explicitly so the emitted
/// text carries no reliance on the reader's precedence table.
#[inline]
pub fn e1_bit_mix_body<C: Cf>(state: C::Uint, ret_out: &C::RetCell) -> Flow {
    // uint rot = state >> 28u;
    let rot = C::temp_uint("rot", C::shr_u(state, C::uint_lit(ROT_SHIFT)));
    // uint word = state ^ (state << 13u);
    let word = C::temp_uint(
        "word",
        C::uxor(state, C::ushl(state, C::uint_lit(XSH_SHIFT))),
    );
    // uint tail = word & 65535u;
    let tail = C::temp_uint("tail", C::and_u(word, C::uint_lit(LOW16_MASK)));
    // return tail | rot;
    C::ret(ret_out, C::uor(tail, rot))
}

/// **E2 (encode)** — packs two `float`s into one `uint` as an IEEE binary16 pair
/// ([`Cf::f32tof16`] + [`Cf::ushl`] + [`Cf::uor`]), depositing the word into `ret_out`.
///
/// ```text
/// uint lo = f32tof16(x);
/// uint hi = f32tof16(y);
/// return lo | (hi << 16u);
/// ```
#[inline]
pub fn e2_pack_half2_body<C: Cf>(x: C::Scalar, y: C::Scalar, ret_out: &C::RetCell) -> Flow {
    let lo = C::temp_uint("lo", C::f32tof16(x));
    let hi = C::temp_uint("hi", C::f32tof16(y));
    // return lo | (hi << 16u);  — the shift wraps (it is a bitwise operand of `|`).
    C::ret(
        ret_out,
        C::uor(lo, C::ushl(hi, C::uint_lit(HALF2_HI_SHIFT))),
    )
}

/// **E2 (decode)** — the inverse of [`e2_pack_half2_body`]: unpacks the binary16 pair out of
/// one `uint` ([`Cf::f16tof32`] + [`Cf::and_u`] + [`Cf::shr_u`]) and deposits their SUM into
/// `ret_out` (a scalar so the probe needs no `float2` return).
///
/// ```text
/// float lo = f16tof32(packed & 65535u);
/// float hi = f16tof32(packed >> 16u);
/// return lo + hi;
/// ```
///
/// The low-half mask is spelled even though `f16tof32` ignores the high 16 bits — the emitted
/// text is what a real leaf writes, and the redundant mask is what makes the decode readable.
#[inline]
pub fn e2_unpack_half2_body<C: Cf>(packed: C::Uint, ret_out: &C::RetCellF) -> Flow {
    let lo = C::temp_float("lo", C::f16tof32(C::and_u(packed, C::uint_lit(LOW16_MASK))));
    let hi = C::temp_float(
        "hi",
        C::f16tof32(C::shr_u(packed, C::uint_lit(HALF2_HI_SHIFT))),
    );
    C::ret_f(ret_out, lo.add(hi))
}

/// **E2 (bit-cast)** — negates `x` through the BIT-REINTERPRET pair ([`Cf::asuint`] /
/// [`Cf::asfloat`]) by flipping the sign bit, depositing the result into `ret_out`.
///
/// ```text
/// uint bits = asuint(x);
/// uint flipped = bits ^ 2147483648u;
/// return asfloat(flipped);
/// ```
///
/// The round trip is EXACT for every input — including `±0` and every NaN payload — precisely
/// because a bit-cast is not a numeric conversion; that is the property this probe pins, and
/// it is why `asuint` must never be confused with [`Cf::float_to_uint`].
#[inline]
pub fn e2_bitcast_sign_flip_body<C: Cf>(x: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    let bits = C::temp_uint("bits", C::asuint(x));
    let flipped = C::temp_uint("flipped", C::uxor(bits, C::uint_lit(F32_SIGN_MASK)));
    C::ret_f(ret_out, C::asfloat(flipped))
}

/// **E3** — the `dot` probe ([`Cf::vec3_dot`]), spelling the intrinsic BOTH as a named temp and
/// inline inside a product, and depositing the scalar into `ret_out`.
///
/// ```text
/// float vn = dot(v, n);
/// return vn * dot(v, v);
/// ```
///
/// The inline use is the one worth pinning: `dot(...)` is a function-call form, so it composes
/// under `*` with NO parentheses.
#[inline]
pub fn e3_dot_body<C: Cf>(v: C::Vec3f, n: C::Vec3f, ret_out: &C::RetCellF) -> Flow {
    let vn = C::temp_float("vn", C::vec3_dot(v, n));
    C::ret_f(ret_out, vn.mul(C::vec3_dot(v, v)))
}

/// **E4** — the trig probe ([`Cf::sin`] / [`Cf::cos`]), depositing the Pythagorean identity
/// into `ret_out`.
///
/// ```text
/// float s = sin(theta);
/// float c = cos(theta);
/// return s * s + c * c;
/// ```
///
/// `sin²+cos² == 1` is a SELF-CHECKING oracle: the Eval side needs no table of expected
/// sine values to prove the two hooks are wired to the right functions, and a swapped
/// `sin`/`cos` pair would still satisfy it — which is why the facet test ALSO pins the emitted
/// text, where a swap is visible.
#[inline]
pub fn e4_trig_body<C: Cf>(theta: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    let s = C::temp_float("s", C::sin(theta));
    let c = C::temp_float("c", C::cos(theta));
    C::ret_f(ret_out, s.mul(s).add(c.mul(c)))
}

/// **E5** — the renormalization probe ([`Cf::rsqrt`]): rescales the stored `(cos, sin)`
/// rotation pair back onto the unit circle after a step, depositing the renormalized cosine
/// into `ret_out`.
///
/// ```text
/// float len_sq = c * c + s * s;
/// float inv_len = rsqrt(len_sq);
/// return c * inv_len;
/// ```
///
/// `rsqrt` — not `1.0 / sqrt(...)` — is the whole point: it is ONE approximate instruction
/// where the spelled-out form is a `sqrt` plus an `OpFDiv`. The scalar DIVIDE needs no facet of
/// its own; it is [`FieldScalar::div`], reachable from any `C: Cf` body through
/// [`Cf::Scalar`]'s trait bound (as [`crate::pack::pack_material_id_ba_body`] spells it).
#[inline]
pub fn e5_renorm_body<C: Cf>(c: C::Scalar, s: C::Scalar, ret_out: &C::RetCellF) -> Flow {
    let len_sq = C::temp_float("len_sq", c.mul(c).add(s.mul(s)));
    let inv_len = C::temp_float("inv_len", C::rsqrt(len_sq));
    C::ret_f(ret_out, c.mul(inv_len))
}
