//! IEEE 754 **binary16** ↔ **binary32** conversion — the Eval oracle behind the
//! [`Cf::f16tof32`](crate::cf::Cf::f16tof32) / [`Cf::f32tof16`](crate::cf::Cf::f32tof16)
//! facets (the packed half-precision attribute decode/encode the particle leaves need).
//!
//! IN-HOUSE: hand-rolled integer bit surgery, ZERO third-party deps (no `half` crate) and
//! `#![no_std]`-clean — the only float operations are [`f32::to_bits`] / [`f32::from_bits`],
//! both `core` and both `const`. The module is therefore linkable from the Eval (physics-leaf)
//! profile exactly like [`crate::scalar`].
//!
//! # Why IEEE-exact (what the GPU actually does)
//!
//! MEASURED against the frozen toolchain (`dxc -T cs_6_0 -spirv`, Vulkan SDK 1.4.350):
//!
//! - `f32tof16(f)` lowers to `OpCompositeConstruct %v2float %f %float_0` +
//!   `OpExtInst %uint GLSL.std.450 PackHalf2x16` — i.e. the GLSL `packHalf2x16`, specified as
//!   the IEEE 754 binary16 conversion with **round-to-nearest-even**, the half in the LOW 16
//!   bits (the second lane is the constant `0.0`, so the high 16 bits are zero).
//! - `f16tof32(u)` lowers to `OpExtInst %v2float GLSL.std.450 UnpackHalf2x16` +
//!   `OpCompositeExtract 0` — the LOW 16 bits of the operand, widened IEEE-exactly.
//!
//! So the host oracle is the plain IEEE conversion: subnormals on BOTH sides are real values
//! (never flushed), overflow produces ±Inf, and a NaN stays a NaN with its payload truncated
//! (see [`f32_to_f16_bits`]). This is a bit-exact contract — the conversion is integer-only,
//! so unlike a division it carries no rounding freedom.

/// Widens the IEEE 754 **binary16** value held in the LOW 16 bits of `bits` to `f32`.
///
/// The HIGH 16 bits are IGNORED — HLSL's `f16tof32` reads a `uint` whose upper half is
/// unspecified (the `OpExtInst UnpackHalf2x16` + `OpCompositeExtract 0` lowering discards
/// lane 1), so a caller may pass a packed pair directly.
///
/// Exact for every input, with no carve-outs:
/// - **±0** (`0x0000` / `0x8000`) → ±0.0, sign preserved;
/// - **subnormal** (biased exponent 0, non-zero significand) → the exact value
///   `significand · 2⁻²⁴`, re-normalized into a binary32 NORMAL (binary32 has the exponent
///   range to represent every binary16 subnormal exactly);
/// - **±Inf** (`0x7C00` / `0xFC00`) → ±[`f32::INFINITY`];
/// - **NaN** → a NaN whose 10-bit payload is shifted left by 13 into the binary32
///   significand (so the quiet bit stays the quiet bit and the payload is preserved).
#[inline]
#[must_use]
pub const fn f16_bits_to_f32(bits: u32) -> f32 {
    let h = bits & 0xFFFF;
    // The sign moves from bit 15 to bit 31.
    let sign = (h & 0x8000) << 16;
    let exp = (h >> 10) & 0x1F;
    let man = h & 0x03FF;

    let out = if exp == 0 {
        if man == 0 {
            // ±0 — the significand carries no value, only the sign survives.
            sign
        } else {
            // Subnormal: the value is `man · 2⁻²⁴`. Shift the significand left until its
            // MSB reaches bit 10, where it becomes binary32's IMPLICIT leading 1 (and is
            // therefore masked back off); each shift step lowers the exponent by one.
            // `man` is in `1..=0x3FF`, so `leading_zeros()` is in `22..=31` and `shift` in
            // `1..=10`.
            let shift = man.leading_zeros() - 21;
            // Unbiased exponent `-14 - shift`, biased by 127 → `113 - shift`.
            let exp32 = 113 - shift;
            sign | (exp32 << 23) | (((man << shift) & 0x03FF) << 13)
        }
    } else if exp == 0x1F {
        // ±Inf (`man == 0`) or NaN — the all-ones exponent maps to binary32's all-ones
        // exponent and the payload widens by 13 bits, so a NaN never decays to an Inf.
        sign | 0x7F80_0000 | (man << 13)
    } else {
        // Normal: re-bias the exponent (127 - 15 = 112) and widen the significand.
        sign | ((exp + 112) << 23) | (man << 13)
    };

    f32::from_bits(out)
}

/// Narrows `x` to IEEE 754 **binary16**, returning the 16 bits in the LOW half of a `u32`
/// (the HIGH 16 bits are always ZERO — the `PackHalf2x16` lowering packs `0.0` into lane 1).
///
/// **Round-to-nearest-even** on the 13 (or more, for a subnormal result) discarded
/// significand bits — the rounding `packHalf2x16` specifies. The full behavior:
/// - **overflow** (|x| above the binary16 range, including the tie at 65520.0 which rounds to
///   the even neighbour) → ±Inf `0x7C00` / `0xFC00`, NOT a clamp to `HALF_MAX`;
/// - **underflow** → a binary16 SUBNORMAL where one is representable (down to `2⁻²⁴`), then
///   ±0 (`2⁻²⁵` is the exact tie and rounds to even, i.e. to zero). A binary32 subnormal input
///   is always far below that and yields ±0 with its sign preserved;
/// - **NaN** → a NaN. The 23-bit payload TRUNCATES to its top 10 bits; when every surviving
///   bit is zero the pattern would read as an Inf, so the quiet bit (`0x0200`) is forced —
///   the conversion must never turn a NaN into an infinity.
#[inline]
#[must_use]
pub const fn f32_to_f16_bits(x: f32) -> u32 {
    let bits = x.to_bits();
    // The sign moves from bit 31 to bit 15.
    let sign = (bits >> 16) & 0x8000;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let man = bits & 0x007F_FFFF;

    if exp == 0xFF {
        if man == 0 {
            return sign | 0x7C00;
        }
        let payload = man >> 13;
        return sign | 0x7C00 | if payload == 0 { 0x0200 } else { payload };
    }

    let unbiased = exp - 127;

    if unbiased > 15 {
        return sign | 0x7C00;
    }

    if unbiased >= -14 {
        // Normal result. A significand carry propagates into the exponent field on its own
        // (the fields are adjacent), and an exponent carry out of `unbiased == 15` lands
        // EXACTLY on the ±Inf pattern `0x7C00` — the IEEE overflow result, for free.
        let h = sign | (((unbiased + 15) as u32) << 10) | (man >> 13);
        return h + rtne_increment(man, 13);
    }

    if unbiased < -25 {
        // Below half of the smallest subnormal — rounds to ±0 under every rounding mode.
        return sign;
    }

    // Subnormal result: restore the implicit leading 1 and drop the extra
    // `-14 - unbiased` bits the fixed subnormal exponent forces (`drop` is 14..=24).
    let m_full = man | 0x0080_0000;
    let drop = (13 + (-14 - unbiased)) as u32;
    sign | ((m_full >> drop) + rtne_increment(m_full, drop))
}

/// The round-to-nearest-**even** increment for dropping the low `drop` bits of `m`: `1` when
/// the discarded remainder is above half an ulp, or exactly half with an ODD surviving LSB;
/// `0` otherwise. `drop` is `1..=24` at every call site, so `1 << drop` cannot overflow.
#[inline]
const fn rtne_increment(m: u32, drop: u32) -> u32 {
    let half = 1u32 << (drop - 1);
    let rem = m & ((1u32 << drop) - 1);
    let keep_lsb = (m >> drop) & 1;
    if rem > half || (rem == half && keep_lsb == 1) {
        1
    } else {
        0
    }
}
