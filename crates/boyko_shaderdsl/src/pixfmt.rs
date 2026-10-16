//! Render wave PX0 (`px0/cut.md` §1.3 #2 and #3) — **the one home of the pixel encodings**: the
//! VkFormat codes the image dumps carry, their texel decoders, the per-channel float encodings
//! (`UFLOAT11`, `UFLOAT10`, binary16, UNORM8) under both rounding modes, the B10G11R11 pack /
//! unpack, the rounding model family and its equivalence classes, and **THE format step** — the
//! integer distance in a channel's encoding that every `lit`-domain gate counts in.
//!
//! # Why here
//!
//! `post_eval` (this crate's bin), `boyko_rhi_vulkan`'s goldens and probe tests, `boyko_app`'s dump
//! reader and `tools/golden_diff.py` all decode the same bits and count the same steps. A home in
//! `boyko_rhi_vulkan` would make this crate depend on it (a cycle: the backend already reaches the
//! eDSL); `boyko_app/tests/common` is importable by nothing. This module is `no_std`, has no
//! feature and no dependency, so every one of them can reach it (OPEN-ITEMS #3; `critique_arch_r5`
//! W12). The Python mirror in `tools/golden_diff.py` is pinned to it by committed vectors
//! (`tests/golden_diff_vectors_sync.rs`).
//!
//! # The format step (`critique_arch_r5` W12, `px0/critique.md` W1)
//!
//! [`steps_between`] and [`step_by`] count representable values, not a relative tolerance, so the
//! "one step at the lower or the upper binade" ambiguity cannot arise. Every encoding is mapped to a
//! **monotone integer line** ([`ordinal`]):
//! - the unsigned encodings (`UFLOAT11`, `UFLOAT10`, UNORM8) — the code itself;
//! - **binary16, the one signed encoding** — the sign-magnitude code mapped to `±magnitude`, so
//!   `−0.0` and `+0.0` are the SAME point (`0`) and a step across zero is one step per
//!   representable value, never ≈ 2¹⁵. `taa_hist` (RGBA16F) is stored unclamped and can hold
//!   negatives and signed zeros (`taa_resolve.comp.hlsl`), which is why the line is signed.
//!
//! [`step_by`] moves toward `+∞` for `k > 0` on every value (a negative value moves toward zero),
//! saturating at the finite ends of the encoding. **A non-finite value (NaN, ±Inf, or a finite
//! value past the encoding's overflow threshold) is a named error** ([`StepError::NonFinite`]), and
//! a negative value given to an unsigned encoding is another ([`StepError::Negative`]); a consumer
//! that walks a dump excludes such pixels by [`pixel_is_nonfinite`], the predicate
//! `post::hdr_sanitize` zeroes a pixel by.
//!
//! # The rounding model family (H3 Scope 3) and its equivalence classes (OPEN-ITEMS #2)
//!
//! [`Member`] is the family the host model evaluates; [`PathClass`] is what a device path can be
//! classified as. B10G11R11's channels are a sub-grid of binary16 (same 5-bit exponent and bias,
//! fewer mantissa bits), so truncating through `f16` equals truncating directly (`Z16 ≡ Z`) and, on
//! a binary16 target, rounding to `f16` first is one rounding twice (`N16 ≡ N`). The classes per
//! (format, path) are [`classes_for`]; `Zpre ≢ Z` (witness in the tests). Overflow, stated: the
//! host model maps a finite value past the largest finite code to `+Inf` under `NearestEven` and to
//! the largest finite code under `TowardZero` — the probe's device inputs stay at or below each
//! channel's largest finite value, so a device's overflow rule never enters classification.

use crate::half::f16_bits_to_f32;

// ---- VkFormat codes ------------------------------------------------------------------------------

/// The dump `format` word of a BUFFER dump (no VkFormat): `width` = byte length, `height = 1`,
/// `layers = 1`, `texel_bytes = 1` ([`crate::dump`]'s buffer contract).
pub const FORMAT_BUFFER: u32 = 0;
/// `VK_FORMAT_R8G8B8A8_UNORM`.
pub const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
/// `VK_FORMAT_R8G8B8A8_SRGB`.
pub const VK_FORMAT_R8G8B8A8_SRGB: u32 = 43;
/// `VK_FORMAT_B8G8R8A8_UNORM`.
pub const VK_FORMAT_B8G8R8A8_UNORM: u32 = 44;
/// `VK_FORMAT_B8G8R8A8_SRGB`.
pub const VK_FORMAT_B8G8R8A8_SRGB: u32 = 50;
/// `VK_FORMAT_R16G16B16A16_SFLOAT`.
pub const VK_FORMAT_R16G16B16A16_SFLOAT: u32 = 97;
/// `VK_FORMAT_R32_SFLOAT`.
pub const VK_FORMAT_R32_SFLOAT: u32 = 100;
/// `VK_FORMAT_R32G32B32A32_SFLOAT`.
pub const VK_FORMAT_R32G32B32A32_SFLOAT: u32 = 109;
/// `VK_FORMAT_B10G11R11_UFLOAT_PACK32`.
pub const VK_FORMAT_B10G11R11_UFLOAT_PACK32: u32 = 122;
/// `VK_FORMAT_D32_SFLOAT`.
pub const VK_FORMAT_D32_SFLOAT: u32 = 126;

/// Every `format` word a dump may carry — the reader's table, which `dump_format_tables_agree`
/// holds equal to the image-dump writer's.
pub const DUMP_FORMATS: [u32; 10] = [
    FORMAT_BUFFER,
    VK_FORMAT_R8G8B8A8_UNORM,
    VK_FORMAT_R8G8B8A8_SRGB,
    VK_FORMAT_B8G8R8A8_UNORM,
    VK_FORMAT_B8G8R8A8_SRGB,
    VK_FORMAT_R16G16B16A16_SFLOAT,
    VK_FORMAT_R32_SFLOAT,
    VK_FORMAT_R32G32B32A32_SFLOAT,
    VK_FORMAT_B10G11R11_UFLOAT_PACK32,
    VK_FORMAT_D32_SFLOAT,
];

/// The texel size of a dump `format`, or `None` for a format this module does not decode.
#[must_use]
pub const fn texel_bytes(format: u32) -> Option<u32> {
    match format {
        FORMAT_BUFFER => Some(1),
        VK_FORMAT_R8G8B8A8_UNORM
        | VK_FORMAT_R8G8B8A8_SRGB
        | VK_FORMAT_B8G8R8A8_UNORM
        | VK_FORMAT_B8G8R8A8_SRGB
        | VK_FORMAT_R32_SFLOAT
        | VK_FORMAT_B10G11R11_UFLOAT_PACK32
        | VK_FORMAT_D32_SFLOAT => Some(4),
        VK_FORMAT_R16G16B16A16_SFLOAT => Some(8),
        VK_FORMAT_R32G32B32A32_SFLOAT => Some(16),
        _ => None,
    }
}

/// One texel as four `f32`s: 8-bit formats are `code / 255` in `[r, g, b, a]` order (BGRA is
/// reordered; an sRGB format's bytes are returned ENCODED, not linearised), binary16 is widened
/// exactly, one-channel formats fill `[v, 0, 0, 1]`, B10G11R11 fills `[r, g, b, 1]`. `None` for a
/// buffer dump, an unknown format, or a slice that is not exactly one texel.
#[must_use]
pub fn decode_texel(format: u32, t: &[u8]) -> Option<[f32; 4]> {
    if Some(t.len() as u32) != texel_bytes(format) {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes([t[i * 4], t[i * 4 + 1], t[i * 4 + 2], t[i * 4 + 3]]);
    let f32_at = |i: usize| f32::from_bits(u32_at(i));
    let h_at = |i: usize| f16_bits_to_f32(u32::from(u16::from_le_bytes([t[i * 2], t[i * 2 + 1]])));
    let u8n = |b: u8| decode(Channel::Unorm8, u32::from(b));
    Some(match format {
        VK_FORMAT_R8G8B8A8_UNORM | VK_FORMAT_R8G8B8A8_SRGB => [u8n(t[0]), u8n(t[1]), u8n(t[2]), u8n(t[3])],
        VK_FORMAT_B8G8R8A8_UNORM | VK_FORMAT_B8G8R8A8_SRGB => [u8n(t[2]), u8n(t[1]), u8n(t[0]), u8n(t[3])],
        VK_FORMAT_R16G16B16A16_SFLOAT => [h_at(0), h_at(1), h_at(2), h_at(3)],
        VK_FORMAT_R32_SFLOAT | VK_FORMAT_D32_SFLOAT => [f32_at(0), 0.0, 0.0, 1.0],
        VK_FORMAT_R32G32B32A32_SFLOAT => [f32_at(0), f32_at(1), f32_at(2), f32_at(3)],
        VK_FORMAT_B10G11R11_UFLOAT_PACK32 => {
            let [r, g, b] = unpack_b10g11r11(u32_at(0));
            [r, g, b, 1.0]
        }
        _ => return None,
    })
}

// ---- Channel encodings -----------------------------------------------------------------------------

/// One channel's encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// `UFLOAT11`: 5-bit exponent (bias 15), 6-bit mantissa, no sign — B10G11R11's R and G.
    Uf11,
    /// `UFLOAT10`: 5-bit exponent (bias 15), 5-bit mantissa, no sign — B10G11R11's B.
    Uf10,
    /// IEEE 754 binary16 — RGBA16F's channels. The one SIGNED encoding.
    F16,
    /// 8-bit UNORM — the display formats.
    Unorm8,
}

impl Channel {
    /// The stored mantissa bits (0 for UNORM8).
    #[must_use]
    pub const fn mantissa_bits(self) -> u32 {
        match self {
            Channel::Uf11 => 6,
            Channel::Uf10 => 5,
            Channel::F16 => 10,
            Channel::Unorm8 => 0,
        }
    }

    /// Whether the encoding carries a sign (binary16 only).
    #[must_use]
    pub const fn is_signed(self) -> bool {
        matches!(self, Channel::F16)
    }

    /// The `+Inf` code of a float encoding (exponent all ones, mantissa 0); UNORM8 has none.
    #[must_use]
    pub const fn inf_code(self) -> Option<u32> {
        match self {
            Channel::Unorm8 => None,
            _ => Some(31 << self.mantissa_bits()),
        }
    }

    /// The largest finite code (UNORM8: 255).
    #[must_use]
    pub const fn max_finite_code(self) -> u32 {
        match self.inf_code() {
            Some(inf) => inf - 1,
            None => 255,
        }
    }

    /// The largest finite value: 65024 (UFLOAT11), 64512 (UFLOAT10), 65504 (binary16), 1 (UNORM8).
    #[must_use]
    pub const fn max_finite(self) -> f32 {
        decode(self, self.max_finite_code())
    }

    /// The short spelling the vectors and `golden_diff.py` use (`uf11`, `uf10`, `f16`, `unorm8`).
    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            Channel::Uf11 => "uf11",
            Channel::Uf10 => "uf10",
            Channel::F16 => "f16",
            Channel::Unorm8 => "unorm8",
        }
    }
}

/// B10G11R11's three channels in `[r, g, b]` order.
pub const B10G11R11_CHANNELS: [Channel; 3] = [Channel::Uf11, Channel::Uf11, Channel::Uf10];

/// A narrowing conversion's rounding mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Round to nearest, ties to even.
    NearestEven,
    /// Round toward zero (truncate).
    TowardZero,
}

/// `2^k` as an `f32` for `k` in the normal range (`-126..=127`) — exact.
const fn pow2(k: i32) -> f32 {
    f32::from_bits(((k + 127) as u32) << 23)
}

/// Encodes `x` into `ch` under `r`.
///
/// Float encodings: NaN → a quiet NaN code; `+Inf` → the Inf code; a negative value (and `−0.0`,
/// and `−Inf`) → `0` on an unsigned encoding (the Vulkan rule for UFLOAT); a finite value past the
/// largest finite code → Inf under `NearestEven`, the largest finite code under `TowardZero`;
/// subnormal results are real values (never flushed). UNORM8: `x` clamped to `[0, 1]`, then
/// `x · 255` rounded (a NaN encodes 0).
#[must_use]
pub const fn encode(ch: Channel, x: f32, r: Rounding) -> u32 {
    match ch {
        Channel::Unorm8 => encode_unorm8(x, r),
        _ => encode_float(x, ch.mantissa_bits(), ch.is_signed(), r),
    }
}

const fn encode_unorm8(x: f32, r: Rounding) -> u32 {
    if x.is_nan() || x <= 0.0 {
        return 0;
    }
    if x >= 1.0 {
        return 255;
    }
    let v = x * 255.0;
    let t = v as u32;
    let frac = v - t as f32;
    match r {
        Rounding::TowardZero => t,
        Rounding::NearestEven => {
            if frac > 0.5 || (frac == 0.5 && t & 1 == 1) {
                t + 1
            } else {
                t
            }
        }
    }
}

const fn encode_float(x: f32, m: u32, signed: bool, r: Rounding) -> u32 {
    let bits = x.to_bits();
    let neg = bits >> 31 != 0;
    let exp = (bits >> 23) & 0xFF;
    let man = bits & 0x007F_FFFF;
    let inf = 31u32 << m;
    let sign_bit = if signed { 1u32 << (m + 5) } else { 0 };
    if exp == 0xFF && man != 0 {
        return (if neg { sign_bit } else { 0 }) | inf | (1 << (m - 1));
    }
    if neg && !signed {
        return 0;
    }
    let s = if neg { sign_bit } else { 0 };
    if exp == 0xFF {
        return s | inf;
    }
    if exp == 0 && man == 0 {
        return s;
    }
    // `x = sig · 2^(e − 23)` with `sig` the 24-bit significand (no implicit bit for an f32
    // subnormal, whose exponent is −126).
    let (sig, e) = if exp == 0 { (man, -126i32) } else { (man | 0x0080_0000, exp as i32 - 127) };
    if e > 15 {
        return s | match r {
            Rounding::NearestEven => inf,
            Rounding::TowardZero => inf - 1,
        };
    }
    // The quantum is `2^(e − m)` in the normal range and `2^(−14 − m)` below it, so the code's
    // mantissa-with-leading-bit is `sig >> sh`.
    let sh = if e >= -14 { 23 - m as i32 } else { 23 - m as i32 + (-14 - e) };
    let q = if sh >= 25 {
        // `sig < 2^24 ≤ half`: below half of the smallest quantum under either mode.
        0
    } else {
        let sh = sh as u32;
        let t = sig >> sh;
        match r {
            Rounding::TowardZero => t,
            Rounding::NearestEven => {
                let rem = sig & ((1 << sh) - 1);
                let half = 1 << (sh - 1);
                if rem > half || (rem == half && t & 1 == 1) { t + 1 } else { t }
            }
        }
    };
    // A mantissa carry (`q == 2^(m+1)`) propagates into the exponent field on its own, and out of
    // `e == 15` it lands exactly on the Inf code — the `NearestEven` overflow, for free.
    let code = if e >= -14 { (((e + 15) as u32) << m) + q - (1 << m) } else { q };
    s | code
}

/// Decodes a code of `ch` (exact).
#[must_use]
pub const fn decode(ch: Channel, code: u32) -> f32 {
    match ch {
        Channel::Unorm8 => (code & 0xFF) as f32 / 255.0,
        _ => {
            let m = ch.mantissa_bits();
            let neg = ch.is_signed() && (code >> (m + 5)) & 1 == 1;
            let e = (code >> m) & 0x1F;
            let mb = code & ((1 << m) - 1);
            let mag = if e == 0 {
                mb as f32 * pow2(-14 - m as i32)
            } else if e == 31 {
                if mb == 0 { f32::INFINITY } else { f32::NAN }
            } else {
                f32::from_bits(((e + 112) << 23) | (mb << (23 - m)))
            };
            if neg { -mag } else { mag }
        }
    }
}

/// Rounds `x` to the nearest value representable in `ch` under `r` (encode, then decode).
#[must_use]
pub const fn quantize(ch: Channel, x: f32, r: Rounding) -> f32 {
    decode(ch, encode(ch, x, r))
}

/// Packs `[r, g, b]` into a B10G11R11 word (R bits 0..11, G bits 11..22, B bits 22..32), each
/// channel rounded under `rounding`.
#[must_use]
pub const fn pack_b10g11r11(rgb: [f32; 3], rounding: Rounding) -> u32 {
    let r = encode(Channel::Uf11, rgb[0], rounding);
    let g = encode(Channel::Uf11, rgb[1], rounding);
    let b = encode(Channel::Uf10, rgb[2], rounding);
    r | (g << 11) | (b << 22)
}

/// Unpacks a B10G11R11 word to `[r, g, b]` (exact) — THE decoder of the format (the goldens
/// re-export it; PX1a reuses it).
#[must_use]
pub const fn unpack_b10g11r11(word: u32) -> [f32; 3] {
    [
        decode(Channel::Uf11, word & 0x7FF),
        decode(Channel::Uf11, (word >> 11) & 0x7FF),
        decode(Channel::Uf10, (word >> 22) & 0x3FF),
    ]
}

// ---- The format step -----------------------------------------------------------------------------

/// Why a value has no place on a channel's step line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepError {
    /// NaN, ±Inf, or a finite value past the encoding's overflow threshold.
    NonFinite,
    /// A negative value given to an unsigned encoding.
    Negative,
}

/// `x`'s position on `ch`'s monotone integer line: the round-to-nearest-even code for the unsigned
/// encodings, `±magnitude` for binary16 (so `−0.0` and `+0.0` are both `0`).
///
/// # Errors
/// [`StepError::NonFinite`] for NaN / ±Inf / an overflowing value; [`StepError::Negative`] for a
/// negative value on an unsigned encoding (`−0.0` is not negative here).
pub const fn ordinal(ch: Channel, x: f32) -> Result<i32, StepError> {
    if x.is_nan() || x.is_infinite() {
        return Err(StepError::NonFinite);
    }
    if !ch.is_signed() && x < 0.0 {
        return Err(StepError::Negative);
    }
    let code = encode(ch, x, Rounding::NearestEven);
    if let Some(inf) = ch.inf_code() {
        let m = ch.mantissa_bits();
        let mag = code & ((1 << (m + 5)) - 1);
        if mag >= inf {
            return Err(StepError::NonFinite);
        }
        let neg = ch.is_signed() && (code >> (m + 5)) & 1 == 1;
        return Ok(if neg { -(mag as i32) } else { mag as i32 });
    }
    Ok(code as i32)
}

/// **THE format step**: how many representable values of `ch` separate `a` and `b`.
///
/// # Errors
/// As [`ordinal`], for either operand.
pub const fn steps_between(ch: Channel, a: f32, b: f32) -> Result<u32, StepError> {
    let oa = match ordinal(ch, a) {
        Ok(o) => o,
        Err(e) => return Err(e),
    };
    let ob = match ordinal(ch, b) {
        Ok(o) => o,
        Err(e) => return Err(e),
    };
    Ok(oa.abs_diff(ob))
}

/// `x` moved by `k` representable values of `ch` — toward `+∞` for `k > 0` on every value (a
/// negative binary16 value moves toward zero), saturating at the encoding's finite ends.
///
/// # Errors
/// As [`ordinal`].
pub const fn step_by(ch: Channel, x: f32, k: i32) -> Result<f32, StepError> {
    let o = match ordinal(ch, x) {
        Ok(o) => o,
        Err(e) => return Err(e),
    };
    let max = ch.max_finite_code() as i32;
    let min = if ch.is_signed() { -max } else { 0 };
    let t = o.saturating_add(k);
    let t = if t > max {
        max
    } else if t < min {
        min
    } else {
        t
    };
    let code = if t < 0 { (1 << (ch.mantissa_bits() + 5)) | (-t) as u32 } else { t as u32 };
    Ok(decode(ch, code))
}

/// Whether a pixel is one `post::hdr_sanitize` zeroes: any channel non-finite (its exponent bits
/// all ones). A step consumer walking a dump excludes such pixels by this predicate.
#[must_use]
pub const fn pixel_is_nonfinite(rgb: [f32; 3]) -> bool {
    nonfinite_bits(rgb[0]) || nonfinite_bits(rgb[1]) || nonfinite_bits(rgb[2])
}

/// `(asuint(x) & F32_EXP_MASK) == F32_EXP_MASK` — `post::hdr_sanitize`'s per-channel test.
const fn nonfinite_bits(x: f32) -> bool {
    x.to_bits() & crate::post::F32_EXP_MASK == crate::post::F32_EXP_MASK
}

// ---- The rounding model family (H3 Scope 3) -------------------------------------------------------

/// One member of the model family: how ONE conversion into the target rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    /// The f32 result rounded once to the format, to nearest-even.
    N,
    /// The same, toward zero.
    Z,
    /// Rounded to binary16 first, then to the format, both to nearest-even.
    N16,
    /// The same, both toward zero.
    Z16,
    /// Blend paths: the source pre-rounded to the format, then the result rounded once (nearest).
    Npre,
    /// The same, toward zero.
    Zpre,
}

impl Member {
    /// Every member.
    pub const ALL: [Member; 6] = [Member::N, Member::Z, Member::N16, Member::Z16, Member::Npre, Member::Zpre];

    /// The member's rounding mode.
    #[must_use]
    pub const fn rounding(self) -> Rounding {
        match self {
            Member::N | Member::N16 | Member::Npre => Rounding::NearestEven,
            Member::Z | Member::Z16 | Member::Zpre => Rounding::TowardZero,
        }
    }

    /// The equivalence class the member belongs to on `target` for `path` — `Z16 ≡ Z` everywhere
    /// (B10's grid is a sub-grid of binary16's, and binary16 onto itself is idempotent), `N16 ≡ N`
    /// on a binary16 target, `Npre ≡ N` / `Zpre ≡ Z` on the single-write paths (no blend source).
    #[must_use]
    pub const fn class(self, target: Target, path: Path) -> PathClass {
        let blend = path.is_blend();
        match self {
            Member::N => PathClass::N,
            Member::Z | Member::Z16 => PathClass::Z,
            Member::N16 => match target {
                Target::B10G11R11 => PathClass::N16,
                Target::Rgba16f => PathClass::N,
            },
            Member::Npre => if blend { PathClass::Npre } else { PathClass::N },
            Member::Zpre => if blend { PathClass::Zpre } else { PathClass::Z },
        }
    }

    /// The representative member of `c` (`None` for [`PathClass::Unmodelled`]).
    #[must_use]
    pub const fn of_class(c: PathClass) -> Option<Member> {
        Some(match c {
            PathClass::N => Member::N,
            PathClass::N16 => Member::N16,
            PathClass::Npre => Member::Npre,
            PathClass::Z => Member::Z,
            PathClass::Zpre => Member::Zpre,
            PathClass::Unmodelled { .. } => return None,
        })
    }

    /// One conversion of the f32 value `v` into `ch` under this member (the `pre` members convert
    /// like their base member when there is no blend source).
    #[must_use]
    pub const fn convert(self, ch: Channel, v: f32) -> u32 {
        let r = self.rounding();
        match self {
            Member::N16 | Member::Z16 => encode(ch, quantize(Channel::F16, v, r), r),
            _ => encode(ch, v, r),
        }
    }

    /// The blend `ONE + ONE` into a target holding `dst` (a code of `ch`): `dst + src`, with the
    /// source pre-rounded to the format under the `pre` members. f32 arithmetic.
    #[must_use]
    pub const fn blend_add(self, ch: Channel, dst: u32, src: f32) -> u32 {
        let d = decode(ch, dst);
        let s = match self {
            Member::Npre | Member::Zpre => quantize(ch, src, self.rounding()),
            _ => src,
        };
        self.convert(ch, s + d)
    }

    /// The premultiplied over `ONE, ONE_MINUS_SRC_ALPHA` into a target holding `dst`:
    /// `src + dst · (1 − alpha)`, f32 arithmetic in that order (the probe's inputs make the product
    /// and the sum exact, so a device's FMA or factor precision cannot move a targeted input).
    #[must_use]
    pub const fn blend_over(self, ch: Channel, dst: u32, src: f32, alpha: f32) -> u32 {
        let d = decode(ch, dst);
        let s = match self {
            Member::Npre | Member::Zpre => quantize(ch, src, self.rounding()),
            _ => src,
        };
        self.convert(ch, s + d * (1.0 - alpha))
    }
}

/// The two `lit` storage candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// `VK_FORMAT_B10G11R11_UFLOAT_PACK32`.
    B10G11R11,
    /// `VK_FORMAT_R16G16B16A16_SFLOAT` (the escape, and `taa_hist`'s format).
    Rgba16f,
}

impl Target {
    /// The channels the probe classifies, `[r, g, b]`.
    #[must_use]
    pub const fn channels(self) -> [Channel; 3] {
        match self {
            Target::B10G11R11 => B10G11R11_CHANNELS,
            Target::Rgba16f => [Channel::F16; 3],
        }
    }

    /// The spelling the record uses (`b10g11r11`, `rgba16f`).
    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            Target::B10G11R11 => "b10g11r11",
            Target::Rgba16f => "rgba16f",
        }
    }
}

/// A path into a target (H3 Scope 3's five).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// Compute storage write.
    Store,
    /// Colour-attachment write (no blend).
    Attachment,
    /// Blend `ONE + ONE`.
    BlendAdd,
    /// Blend `ONE, ONE_MINUS_SRC_ALPHA` (premultiplied over).
    BlendOver,
    /// Clear.
    Clear,
}

impl Path {
    /// The five paths, in the order of the `[PathClass; 5]` rounding constants.
    pub const ALL: [Path; 5] = [Path::Store, Path::Attachment, Path::BlendAdd, Path::BlendOver, Path::Clear];

    /// Whether the path blends a source into the target.
    #[must_use]
    pub const fn is_blend(self) -> bool {
        matches!(self, Path::BlendAdd | Path::BlendOver)
    }

    /// The spelling the record uses.
    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            Path::Store => "store",
            Path::Attachment => "attachment",
            Path::BlendAdd => "blend_add",
            Path::BlendOver => "blend_over",
            Path::Clear => "clear",
        }
    }
}

/// What a device path classified as: one equivalence class of the family, or `Unmodelled` with the
/// largest error it showed (in thousandths of a target ULP, so the type stays `Eq` and `const`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathClass {
    /// Nearest-even, once.
    N,
    /// Through binary16 first, both nearest-even (B10G11R11 only — on RGBA16F it is `N`).
    N16,
    /// Blend source pre-rounded, nearest-even.
    Npre,
    /// Toward zero (`Z16` is the same class).
    Z,
    /// Blend source pre-rounded, toward zero.
    Zpre,
    /// No class matched every output; the largest error, in milli-ULP.
    Unmodelled {
        /// The largest |error| the path showed, in thousandths of a ULP of the target channel.
        max_err_milli_ulp: u32,
    },
}

impl PathClass {
    /// The spelling the record and `golden_diff.py --rounding` use: `N`, `N16`, `Npre`, `Z`,
    /// `Zpre`, `Unmodelled` (the error is spelled `Unmodelled:<ulp>` by [`Self::write_spelling`]).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            PathClass::N => "N",
            PathClass::N16 => "N16",
            PathClass::Npre => "Npre",
            PathClass::Z => "Z",
            PathClass::Zpre => "Zpre",
            PathClass::Unmodelled { .. } => "Unmodelled",
        }
    }

    /// Writes the full spelling (`Unmodelled:<ulp>` with the ULP to three decimals).
    ///
    /// # Errors
    /// The writer's.
    pub fn write_spelling(self, w: &mut impl core::fmt::Write) -> core::fmt::Result {
        match self {
            PathClass::Unmodelled { max_err_milli_ulp } => {
                write!(w, "Unmodelled:{}.{:03}", max_err_milli_ulp / 1000, max_err_milli_ulp % 1000)
            }
            c => w.write_str(c.name()),
        }
    }

    /// Parses a spelling [`Self::write_spelling`] wrote.
    #[must_use]
    pub fn parse(s: &str) -> Option<PathClass> {
        Some(match s {
            "N" => PathClass::N,
            "N16" => PathClass::N16,
            "Npre" => PathClass::Npre,
            "Z" => PathClass::Z,
            "Zpre" => PathClass::Zpre,
            _ => {
                let ulp = s.strip_prefix("Unmodelled:")?;
                let (int, frac) = ulp.split_once('.').unwrap_or((ulp, "0"));
                if frac.len() > 3 || frac.is_empty() {
                    return None;
                }
                let mut milli: u32 = int.parse::<u32>().ok()?.checked_mul(1000)?;
                let scale = [100u32, 10, 1];
                for (i, c) in frac.bytes().enumerate() {
                    if !c.is_ascii_digit() {
                        return None;
                    }
                    milli = milli.checked_add(u32::from(c - b'0') * scale[i])?;
                }
                PathClass::Unmodelled { max_err_milli_ulp: milli }
            }
        })
    }

    /// The per-conversion bound, in ULPs of `ch`, of a single write under this class (AD-19 (a), the
    /// store / attachment / clear paths): `(lo, hi)` with `stored − exact ∈ [lo·ULP, hi·ULP]`.
    /// `N` ±½; `N16` ±(½ + 2^(m−11)) (0.516 UFLOAT10, 0.531 UFLOAT11; ±½ on binary16, where it is
    /// `N`); `Z` [−1, 0]; `Unmodelled` ± its recorded maximum. The `pre` classes bound like their
    /// base (a single write has no blend source).
    #[must_use]
    pub fn store_bound_ulp(self, ch: Channel) -> (f32, f32) {
        match self {
            PathClass::N | PathClass::Npre => (-0.5, 0.5),
            PathClass::N16 => {
                if ch == Channel::F16 {
                    (-0.5, 0.5)
                } else {
                    let e = 0.5 + pow2(ch.mantissa_bits() as i32 - 11);
                    (-e, e)
                }
            }
            PathClass::Z | PathClass::Zpre => (-1.0, 0.0),
            PathClass::Unmodelled { max_err_milli_ulp } => {
                let e = max_err_milli_ulp as f32 / 1000.0;
                (-e, e)
            }
        }
    }
}

/// The equivalence classes of `(target, path)` — the classes a device path can classify as.
#[must_use]
pub const fn classes_for(target: Target, path: Path) -> &'static [PathClass] {
    match (target, path.is_blend()) {
        (Target::B10G11R11, false) => &[PathClass::N, PathClass::N16, PathClass::Z],
        (Target::B10G11R11, true) => {
            &[PathClass::N, PathClass::N16, PathClass::Npre, PathClass::Z, PathClass::Zpre]
        }
        (Target::Rgba16f, false) => &[PathClass::N, PathClass::Z],
        (Target::Rgba16f, true) => &[PathClass::N, PathClass::Npre, PathClass::Z, PathClass::Zpre],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::half::f32_to_f16_bits;

    const BOTH: [Rounding; 2] = [Rounding::NearestEven, Rounding::TowardZero];

    /// Every finite code of `ch` (both signs for binary16).
    fn finite_codes(ch: Channel) -> Vec<u32> {
        let max = ch.max_finite_code();
        let mut v: Vec<u32> = (0..=max).collect();
        if ch.is_signed() {
            v.extend((0..=max).map(|c| c | 0x8000));
        }
        v
    }

    fn next_up(x: f32) -> f32 {
        f32::from_bits(x.to_bits() + 1)
    }

    fn next_down(x: f32) -> f32 {
        if x == 0.0 { 0.0 } else { f32::from_bits(x.to_bits() - 1) }
    }

    /// Exhaustive: every finite code of UFLOAT11, UFLOAT10 and binary16 decodes and re-encodes to
    /// itself under both modes (a representable value never rounds).
    #[test]
    fn every_finite_code_round_trips_under_both_modes() {
        for ch in [Channel::Uf11, Channel::Uf10, Channel::F16] {
            for c in finite_codes(ch) {
                let v = decode(ch, c);
                for r in BOTH {
                    assert_eq!(encode(ch, v, r), c, "{ch:?} code {c:#x} = {v:e} under {r:?}");
                }
            }
        }
        for c in 0..=255u32 {
            assert_eq!(encode(Channel::Unorm8, decode(Channel::Unorm8, c), Rounding::NearestEven), c);
        }
    }

    /// The binary16 encoder agrees with `half.rs` (the f16 encoder the shaders' oracle uses) on
    /// every code, its f32 neighbours and every midpoint, both signs.
    #[test]
    fn f16_nearest_even_matches_half_rs() {
        for c in 0..0x7C00u32 {
            let v = decode(Channel::F16, c);
            let mid = if c < 0x7BFF { (v + decode(Channel::F16, c + 1)) * 0.5 } else { v };
            for x in [v, next_up(v), next_down(v), mid, -v, -mid] {
                assert_eq!(encode(Channel::F16, x, Rounding::NearestEven), f32_to_f16_bits(x), "x = {x:e}");
            }
        }
    }

    /// Known values: 1.0, the largest finite, the smallest subnormal, the smallest normal.
    #[test]
    fn known_codes() {
        assert_eq!(encode(Channel::Uf11, 1.0, Rounding::NearestEven), 15 << 6);
        assert_eq!(encode(Channel::Uf10, 1.0, Rounding::NearestEven), 15 << 5);
        assert_eq!(Channel::Uf11.max_finite(), 65024.0);
        assert_eq!(Channel::Uf10.max_finite(), 64512.0);
        assert_eq!(Channel::F16.max_finite(), 65504.0);
        assert_eq!(decode(Channel::Uf11, 1), 2f32.powi(-20));
        assert_eq!(decode(Channel::Uf10, 1), 2f32.powi(-19));
        assert_eq!(decode(Channel::Uf11, 1 << 6), 2f32.powi(-14));
        assert_eq!(decode(Channel::Uf10, 31 << 5), f32::INFINITY);
        assert!(decode(Channel::Uf10, (31 << 5) | 1).is_nan());
    }

    /// Ties go to even under `NearestEven`; `TowardZero` truncates.
    #[test]
    fn midpoints_round_to_even_and_truncation_truncates() {
        let u = 2f32.powi(-6); // UFLOAT11's ULP in [1, 2)
        let lo_mid = 1.0 + u / 2.0; // between code 960 (even) and 961
        assert_eq!(encode(Channel::Uf11, lo_mid, Rounding::NearestEven), 960);
        assert_eq!(encode(Channel::Uf11, lo_mid, Rounding::TowardZero), 960);
        let hi_mid = 1.0 + 1.5 * u; // between 961 and 962 (even)
        assert_eq!(encode(Channel::Uf11, hi_mid, Rounding::NearestEven), 962);
        assert_eq!(encode(Channel::Uf11, hi_mid, Rounding::TowardZero), 961);
        assert_eq!(encode(Channel::Uf11, next_up(lo_mid), Rounding::NearestEven), 961);
    }

    /// The unsigned encodings take no sign; overflow is Inf under nearest, the top code under
    /// truncation; NaN stays NaN.
    #[test]
    fn negatives_overflow_and_nan_on_unsigned() {
        for ch in [Channel::Uf11, Channel::Uf10] {
            let inf = ch.inf_code().expect("a float encoding");
            for r in BOTH {
                assert_eq!(encode(ch, -1.0, r), 0);
                assert_eq!(encode(ch, -0.0, r), 0);
                assert_eq!(encode(ch, f32::NEG_INFINITY, r), 0);
                assert_eq!(encode(ch, f32::INFINITY, r), inf);
                assert!(decode(ch, encode(ch, f32::NAN, r)).is_nan());
            }
            assert_eq!(encode(ch, 1.0e6, Rounding::NearestEven), inf);
            assert_eq!(encode(ch, 1.0e6, Rounding::TowardZero), inf - 1);
            assert_eq!(encode(ch, ch.max_finite(), Rounding::NearestEven), inf - 1);
        }
        // 65024 + 256 is the tie between UFLOAT11's top code (65024, odd mantissa) and 2^16:
        // ties-to-even goes to 2^16, which is the Inf code.
        assert_eq!(encode(Channel::Uf11, 65024.0 + 256.0, Rounding::NearestEven), 31 << 6);
        assert_eq!(encode(Channel::Uf11, next_down(65024.0 + 256.0), Rounding::NearestEven), (31 << 6) - 1);
    }

    /// The pack layout (R low, B high) and the exhaustive per-channel round trip through it. An R/B
    /// swap in either half reds here.
    #[test]
    fn pack_unpack_layout_and_round_trip() {
        assert_eq!(pack_b10g11r11([1.0, 0.0, 0.0], Rounding::NearestEven), 960);
        assert_eq!(pack_b10g11r11([0.0, 1.0, 0.0], Rounding::NearestEven), 960 << 11);
        assert_eq!(pack_b10g11r11([0.0, 0.0, 1.0], Rounding::NearestEven), 480 << 22);
        for c in finite_codes(Channel::Uf11) {
            let b = c % (Channel::Uf10.max_finite_code() + 1);
            let word = c | ((Channel::Uf11.max_finite_code() - c) << 11) | (b << 22);
            let rgb = unpack_b10g11r11(word);
            assert_eq!(pack_b10g11r11(rgb, Rounding::NearestEven), word, "word {word:#x}");
            assert_eq!(pack_b10g11r11(rgb, Rounding::TowardZero), word, "word {word:#x}");
        }
    }

    /// **`Z16 ≡ Z`** (OPEN-ITEMS #2): over every binary16 code and its f32 neighbours, truncating
    /// through binary16 equals truncating directly, on both B10 channels and on binary16 itself;
    /// and **`N16 ≡ N` on a binary16 target**.
    #[test]
    fn z16_equals_z_on_the_exhaustive_f16_sweep() {
        for c in 0..0x7C00u32 {
            let v = decode(Channel::F16, c);
            for x in [v, next_up(v), next_down(v)] {
                for ch in [Channel::Uf11, Channel::Uf10, Channel::F16] {
                    assert_eq!(Member::Z16.convert(ch, x), Member::Z.convert(ch, x), "{ch:?} x = {x:e}");
                }
                assert_eq!(Member::N16.convert(Channel::F16, x), Member::N.convert(Channel::F16, x), "x = {x:e}");
            }
        }
    }

    /// **`N16 ≢ N` on B10** — a value just above a B10 midpoint, by less than half a binary16 ULP,
    /// rounds up directly but onto the midpoint through binary16 and then ties to even downward.
    #[test]
    fn n16_differs_from_n_on_the_half_f16_ulp_witnesses() {
        for (ch, u) in [(Channel::Uf10, 2f32.powi(-5)), (Channel::Uf11, 2f32.powi(-6))] {
            let mid = 1.0 + u / 2.0; // ties to the even code 1.0 below
            let x = mid + 2f32.powi(-12); // under half a binary16 ULP (2^-11) above the midpoint
            assert_eq!(Member::N.convert(ch, x), encode(ch, 1.0 + u, Rounding::NearestEven), "{ch:?}");
            assert_eq!(Member::N16.convert(ch, x), encode(ch, 1.0, Rounding::NearestEven), "{ch:?}");
        }
    }

    /// **`Zpre ≢ Z`** — the cut's witness on the blue channel.
    #[test]
    fn zpre_differs_from_z_witness() {
        let dst = encode(Channel::Uf10, 2f32.powi(-11), Rounding::NearestEven);
        let src = 1.0 + 1.0 / 32.0 - 2f32.powi(-12);
        assert_eq!(decode(Channel::Uf10, Member::Z.blend_add(Channel::Uf10, dst, src)), 1.0 + 1.0 / 32.0);
        assert_eq!(decode(Channel::Uf10, Member::Zpre.blend_add(Channel::Uf10, dst, src)), 1.0);
    }

    /// THE step: binade edges, subnormals, the top of the range, saturation.
    #[test]
    fn steps_count_representable_values() {
        assert_eq!(steps_between(Channel::Uf10, 1.0, 2.0), Ok(32));
        assert_eq!(steps_between(Channel::Uf11, 1.0, 2.0), Ok(64));
        // Across a binade edge one step is one code, whichever side's ULP.
        assert_eq!(steps_between(Channel::Uf11, 2.0 - 2f32.powi(-6), 2.0), Ok(1));
        assert_eq!(steps_between(Channel::Uf11, 2.0, 2.0 + 2f32.powi(-5)), Ok(1));
        // UFLOAT10's subnormal range holds 32 codes below 2^-14.
        assert_eq!(steps_between(Channel::Uf10, 0.0, 2f32.powi(-14)), Ok(32));
        assert_eq!(step_by(Channel::Uf11, 65024.0, 1), Ok(65024.0));
        assert_eq!(step_by(Channel::Uf11, 0.0, -1), Ok(0.0));
        assert_eq!(step_by(Channel::Uf10, 1.0, 3), Ok(1.0 + 3.0 / 32.0));
        assert_eq!(step_by(Channel::Uf10, 2.0, -1), Ok(2.0 - 1.0 / 32.0));
        assert_eq!(steps_between(Channel::Unorm8, 0.0, 1.0), Ok(255));
    }

    /// **W1 — the signed line.** `−0` and `+0` are one point; a step across zero is one value; a
    /// negative value moves toward zero for `k > 0`.
    #[test]
    fn f16_step_line_is_signed_and_zero_is_one_point() {
        let tiny = 2f32.powi(-24);
        assert_eq!(steps_between(Channel::F16, -0.0, 0.0), Ok(0));
        assert_eq!(steps_between(Channel::F16, -tiny, tiny), Ok(2));
        assert_eq!(steps_between(Channel::F16, -1.0, 1.0), Ok(2 * 0x3C00));
        assert_eq!(step_by(Channel::F16, -1.0, 1), Ok(-(1.0 - 2f32.powi(-11))));
        assert_eq!(step_by(Channel::F16, -0.0, 1), Ok(tiny));
        assert_eq!(step_by(Channel::F16, 0.0, -1), Ok(-tiny));
        assert_eq!(step_by(Channel::F16, -65504.0, -1), Ok(-65504.0));
    }

    /// Non-finite and negative-on-unsigned values are named errors, never a number.
    #[test]
    fn non_finite_and_negative_are_named_errors() {
        assert_eq!(ordinal(Channel::F16, f32::NAN), Err(StepError::NonFinite));
        assert_eq!(ordinal(Channel::F16, f32::INFINITY), Err(StepError::NonFinite));
        assert_eq!(ordinal(Channel::F16, 70_000.0), Err(StepError::NonFinite));
        assert_eq!(ordinal(Channel::Uf11, -1.0), Err(StepError::Negative));
        assert_eq!(ordinal(Channel::Uf11, -0.0), Ok(0));
        assert_eq!(steps_between(Channel::Uf10, 1.0, f32::NAN), Err(StepError::NonFinite));
        assert!(pixel_is_nonfinite([0.0, f32::INFINITY, 0.0]));
        assert!(!pixel_is_nonfinite([0.0, -1.0, 65504.0]));
    }

    /// The predicate is the one `post::hdr_sanitize` zeroes a pixel by.
    #[test]
    fn nonfinite_predicate_is_hdr_sanitize_s() {
        use crate::cf::EvalCf;
        for px in [[1.0, 2.0, 3.0], [f32::NAN, 1.0, 1.0], [1.0, f32::INFINITY, 1.0], [0.5, 0.5, f32::NEG_INFINITY]] {
            let out = crate::post::hdr_sanitize::<EvalCf>(px);
            assert_eq!(out == [0.0; 3], pixel_is_nonfinite(px), "{px:?}");
        }
    }

    /// The texel decoders, per format.
    #[test]
    fn decode_texel_per_format() {
        let w = pack_b10g11r11([1.0, 2.0, 0.5], Rounding::NearestEven).to_le_bytes();
        assert_eq!(decode_texel(VK_FORMAT_B10G11R11_UFLOAT_PACK32, &w), Some([1.0, 2.0, 0.5, 1.0]));
        assert_eq!(decode_texel(VK_FORMAT_R8G8B8A8_UNORM, &[255, 0, 51, 255]), Some([1.0, 0.0, 0.2, 1.0]));
        assert_eq!(decode_texel(VK_FORMAT_B8G8R8A8_UNORM, &[255, 0, 51, 255]), Some([0.2, 0.0, 1.0, 1.0]));
        let h: Vec<u8> = [0x3C00u16, 0xBC00, 0x8000, 0x7BFF].iter().flat_map(|v| v.to_le_bytes()).collect();
        let got = decode_texel(VK_FORMAT_R16G16B16A16_SFLOAT, &h).expect("rgba16f");
        assert_eq!(got.map(f32::to_bits), [1.0f32, -1.0, -0.0, 65504.0].map(f32::to_bits));
        assert_eq!(decode_texel(VK_FORMAT_R32_SFLOAT, &1.25f32.to_le_bytes()), Some([1.25, 0.0, 0.0, 1.0]));
        assert_eq!(decode_texel(VK_FORMAT_R8G8B8A8_UNORM, &[0; 3]), None, "a short slice");
        assert_eq!(decode_texel(FORMAT_BUFFER, &[0]), None, "a buffer has no texel decode");
        assert_eq!(decode_texel(999, &[0; 4]), None, "an unknown format");
        for f in DUMP_FORMATS {
            assert!(texel_bytes(f).is_some(), "format {f} in the table has a size");
        }
    }

    /// The class table is the cut's (§1.3 #2), and the member → class map agrees with it.
    #[test]
    fn classes_table_and_member_map() {
        assert_eq!(classes_for(Target::B10G11R11, Path::Store).len(), 3);
        assert_eq!(classes_for(Target::B10G11R11, Path::BlendOver).len(), 5);
        assert_eq!(classes_for(Target::Rgba16f, Path::Clear).len(), 2);
        assert_eq!(classes_for(Target::Rgba16f, Path::BlendAdd).len(), 4);
        for t in [Target::B10G11R11, Target::Rgba16f] {
            for p in Path::ALL {
                for m in Member::ALL {
                    assert!(classes_for(t, p).contains(&m.class(t, p)), "{m:?} on {t:?}/{p:?}");
                }
                for c in classes_for(t, p) {
                    let rep = Member::of_class(*c).expect("a modelled class");
                    assert_eq!(rep.class(t, p), *c);
                }
            }
        }
    }

    /// The per-class single-write bounds of AD-19 (a).
    #[test]
    fn store_bounds_per_class() {
        assert_eq!(PathClass::N16.store_bound_ulp(Channel::Uf10), (-0.515_625, 0.515_625));
        assert_eq!(PathClass::N16.store_bound_ulp(Channel::Uf11), (-0.531_25, 0.531_25));
        assert_eq!(PathClass::Z.store_bound_ulp(Channel::Uf11), (-1.0, 0.0));
        assert_eq!(PathClass::N.store_bound_ulp(Channel::F16), (-0.5, 0.5));
        let u = PathClass::Unmodelled { max_err_milli_ulp: 750 };
        assert_eq!(u.store_bound_ulp(Channel::Uf10), (-0.75, 0.75));
    }

    /// The class spellings round-trip.
    #[test]
    fn path_class_spelling_round_trips() {
        let all = [
            PathClass::N,
            PathClass::N16,
            PathClass::Npre,
            PathClass::Z,
            PathClass::Zpre,
            PathClass::Unmodelled { max_err_milli_ulp: 1_250 },
        ];
        for c in all {
            let mut s = String::new();
            c.write_spelling(&mut s).expect("a String writes");
            assert_eq!(PathClass::parse(&s), Some(c), "{s}");
        }
        assert_eq!(PathClass::parse("Unmodelled:0.5"), Some(PathClass::Unmodelled { max_err_milli_ulp: 500 }));
        assert_eq!(PathClass::parse("n"), None);
        assert_eq!(PathClass::parse("Unmodelled:1.2345"), None);
    }
}
