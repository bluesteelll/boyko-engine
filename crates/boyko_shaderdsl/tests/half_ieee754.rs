//! The IEEE 754 **binary16** conversion vectors backing the eDSL's `f16tof32` / `f32tof16`
//! facets (rung E2) — [`boyko_shaderdsl::half`].
//!
//! UN-GATED: the conversion is backend-independent and `no_std`-clean, so it is exercised by
//! the DEFAULT (non-`emit`) profile too — the profile a physics build links. The rung-E facet
//! pins that need the HLSL printer live in `tests/particle_facets.rs`.
//!
//! # Where the expected values come from
//!
//! Each vector below was computed by an INDEPENDENT reference — an exact-rational
//! (`fractions.Fraction`) search for the nearest binary16 with round-to-nearest-even — not by
//! running the implementation under test and recording what it said. That is what lets a
//! wrong-but-self-consistent conversion fail here.
//!
//! The behaviour being pinned is the one MEASURED off the frozen toolchain: DXC lowers
//! `f32tof16`/`f16tof32` to `GLSL.std.450 PackHalf2x16`/`UnpackHalf2x16`, i.e. the plain IEEE
//! conversion — real subnormals on both sides, ±Inf on overflow, NaN preserved as NaN.

use boyko_shaderdsl::half::{f16_bits_to_f32, f32_to_f16_bits};

// The boundary values, written as EXACT quotients of powers of two rather than decimal
// literals. Every one of these is exactly representable in binary32 (numerator, denominator
// and quotient alike), so the constant IS the value — whereas the decimal expansion of, say,
// `1023 · 2⁻²⁴` is 24 digits long and a truncated version of it names a DIFFERENT number than
// the one the test is about. Naming the power directly is also what makes a wrong vector
// legible: `1023 / 2²⁴` says "the largest subnormal" where `6.097_555e-5` says nothing.

/// `2⁻¹⁴` — the smallest binary16 NORMAL (`0x0400`).
const HALF_MIN_NORMAL: f32 = 1.0 / 16384.0;
/// `1023 · 2⁻²⁴` — the largest binary16 SUBNORMAL (`0x03FF`).
const HALF_MAX_SUBNORMAL: f32 = 1023.0 / 16_777_216.0;
/// `2⁻²⁴` — the smallest binary16 subnormal (`0x0001`).
const HALF_MIN_SUBNORMAL: f32 = 1.0 / 16_777_216.0;
/// `2⁻²⁵` — exactly HALF of the smallest subnormal, i.e. the round-to-even tie against zero.
const HALF_UNDERFLOW_TIE: f32 = 1.0 / 33_554_432.0;
/// `1.5 · 2⁻²⁵` — above that tie, so it rounds UP to the smallest subnormal.
const HALF_UNDERFLOW_ABOVE_TIE: f32 = 1.5 / 33_554_432.0;

/// The binary32 bit pattern of `x`, for comparisons where `±0` and NaN payloads matter.
fn bits(x: f32) -> u32 {
    x.to_bits()
}

#[test]
fn f32_to_f16_normals_and_zeros() {
    assert_eq!(f32_to_f16_bits(0.0), 0x0000);
    assert_eq!(
        f32_to_f16_bits(-0.0),
        0x8000,
        "the sign of a zero must survive"
    );
    assert_eq!(f32_to_f16_bits(1.0), 0x3C00);
    assert_eq!(f32_to_f16_bits(-2.0), 0xC000);
    assert_eq!(f32_to_f16_bits(0.5), 0x3800);
    assert_eq!(f32_to_f16_bits(123.456), 0x57B7);
    // HALF_MAX, the largest finite binary16.
    assert_eq!(f32_to_f16_bits(65504.0), 0x7BFF);
}

#[test]
fn f32_to_f16_rounds_to_nearest_even() {
    // 1 + 2^-11 is the EXACT midpoint between 1.0 (`0x3C00`, even) and the next half
    // (`0x3C01`, odd) — round-half-to-even keeps 1.0. A round-half-away implementation
    // returns 0x3C01 here, which is the whole reason this vector exists.
    assert_eq!(f32_to_f16_bits(1.0 + 1.0 / 2048.0), 0x3C00);
    // Just above the midpoint (1 + 3·2^-12) rounds up.
    assert_eq!(f32_to_f16_bits(1.0 + 3.0 / 4096.0), 0x3C01);
    // 1 + 2^-10 is exactly representable — no rounding at all.
    assert_eq!(f32_to_f16_bits(1.0 + 1.0 / 1024.0), 0x3C01);
}

#[test]
fn f32_to_f16_overflows_to_infinity() {
    // 65520.0 is the exact tie between HALF_MAX (0x7BFF, odd) and the would-be 65536
    // (0x7C00 = Inf, even) — ties-to-even therefore OVERFLOWS to +Inf. A clamp-to-HALF_MAX
    // implementation returns 0x7BFF and fails here.
    assert_eq!(f32_to_f16_bits(65520.0), 0x7C00);
    assert_eq!(f32_to_f16_bits(-65520.0), 0xFC00);
    // Just below the tie stays finite.
    assert_eq!(f32_to_f16_bits(65519.99), 0x7BFF);
    assert_eq!(f32_to_f16_bits(f32::INFINITY), 0x7C00);
    assert_eq!(f32_to_f16_bits(f32::NEG_INFINITY), 0xFC00);
    assert_eq!(f32_to_f16_bits(f32::MAX), 0x7C00);
}

#[test]
fn f32_to_f16_subnormals_are_real_values() {
    assert_eq!(f32_to_f16_bits(HALF_MIN_NORMAL), 0x0400);
    // A flush-to-zero implementation returns 0x0000 for this and the next two vectors.
    assert_eq!(f32_to_f16_bits(HALF_MAX_SUBNORMAL), 0x03FF);
    assert_eq!(f32_to_f16_bits(HALF_MIN_SUBNORMAL), 0x0001);
    assert_eq!(f32_to_f16_bits(HALF_UNDERFLOW_ABOVE_TIE), 0x0001);
    // Ties-to-even at exactly half the smallest subnormal rounds DOWN to zero.
    assert_eq!(f32_to_f16_bits(HALF_UNDERFLOW_TIE), 0x0000);
    // A binary32 subnormal (and anything else far below the band) underflows, sign intact.
    assert_eq!(f32_to_f16_bits(1e-10), 0x0000);
    assert_eq!(f32_to_f16_bits(-1e-10), 0x8000);
    assert_eq!(f32_to_f16_bits(f32::from_bits(0x0000_0001)), 0x0000);
}

#[test]
fn f32_to_f16_nan_stays_nan_with_truncated_payload() {
    // A quiet NaN: the top 10 payload bits survive (0x40_0001 >> 13 == 0x200).
    assert_eq!(f32_to_f16_bits(f32::from_bits(0x7FC0_0001)), 0x7E00);
    assert_eq!(f32_to_f16_bits(f32::from_bits(0xFFC0_0000)), 0xFE00);
    // THE case worth a test of its own: a signalling NaN whose entire payload lies BELOW the
    // 13 truncated bits. Dropping them naively yields the ±Inf pattern — the conversion must
    // force the quiet bit instead, because a NaN may never become an infinity.
    let narrowed = f32_to_f16_bits(f32::from_bits(0x7F80_0001));
    assert_eq!(
        narrowed, 0x7E00,
        "a NaN whose payload truncates away must stay a NaN"
    );
    assert!(
        f16_bits_to_f32(narrowed).is_nan(),
        "the narrowed pattern must widen back to a NaN, not an infinity"
    );
}

#[test]
fn f16_to_f32_widens_exactly() {
    assert_eq!(bits(f16_bits_to_f32(0x0000)), bits(0.0));
    assert_eq!(
        bits(f16_bits_to_f32(0x8000)),
        bits(-0.0),
        "the sign of a zero must survive"
    );
    assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
    assert_eq!(f16_bits_to_f32(0xC000), -2.0);
    assert_eq!(f16_bits_to_f32(0x7BFF), 65504.0);
    // `0x3555` is 1365 · 2⁻¹² — the nearest binary16 to 1/3, which is what makes it a useful
    // vector: the widened value must be that exact dyadic rational, NOT 1/3.
    assert_eq!(f16_bits_to_f32(0x3555), 1365.0 / 4096.0);
    // Subnormals widen to exact binary32 NORMALS (binary32 has the exponent range for them).
    assert_eq!(f16_bits_to_f32(0x0001), HALF_MIN_SUBNORMAL);
    assert_eq!(f16_bits_to_f32(0x03FF), HALF_MAX_SUBNORMAL);
    assert_eq!(f16_bits_to_f32(0x0400), HALF_MIN_NORMAL);
    assert!(f16_bits_to_f32(0x7C00).is_infinite() && f16_bits_to_f32(0x7C00) > 0.0);
    assert!(f16_bits_to_f32(0xFC00).is_infinite() && f16_bits_to_f32(0xFC00) < 0.0);
    assert!(f16_bits_to_f32(0x7E00).is_nan());
}

#[test]
fn f16_to_f32_ignores_the_high_half() {
    // HLSL's `f16tof32` reads the LOW 16 bits (the `UnpackHalf2x16` + `OpCompositeExtract 0`
    // lowering discards lane 1), so a packed pair may be passed whole.
    assert_eq!(f16_bits_to_f32(0xDEAD_3C00), 1.0);
    assert_eq!(f16_bits_to_f32(0xFFFF_0000), 0.0);
}

#[test]
fn round_trip_is_the_identity_on_every_binary16() {
    // Widening then narrowing must return the ORIGINAL 16 bits for every finite pattern:
    // binary32 represents each one exactly, so no rounding can occur on the way back. This
    // sweeps all 63488 finite patterns (both signs), which is the strongest statement
    // available without a second implementation.
    for h in 0u32..=0xFFFF {
        let exp = (h >> 10) & 0x1F;
        if exp == 0x1F {
            continue; // Inf/NaN are checked by name above (NaN payloads are not a bijection).
        }
        let back = f32_to_f16_bits(f16_bits_to_f32(h));
        assert_eq!(back, h, "round trip changed 0x{h:04X} into 0x{back:04X}");
    }
}
