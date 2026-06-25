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
