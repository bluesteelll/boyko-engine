//! `boyko_shaderdsl` — the in-house Rust shader eDSL (Pass 1: CPU-verifiable).
//!
//! Author the SDF field math ONCE, generic over a [`FieldScalar`](scalar::FieldScalar)
//! backend, and instantiate it two ways:
//!
//! - `S = f32` — the **Eval** backend ([`scalar`]): every op is a single `core`
//!   f32 instruction, BYTE-IDENTICAL to the hand-written `boyko_sdf_math` field
//!   (`boyko_sdf_math` DELEGATES to [`field`] over `f32`).
//! - `S = Emit` — the **HLSL SSA recorder** ([`emit`], `feature = "emit"`): each op
//!   pushes one SSA node; the printer walks the arena into HLSL textually
//!   equivalent to the frozen `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli`.
//!
//! This kills the HLSL↔Rust duplication that caused ~5 field-drift bugs: the field
//! math now lives in ONE place ([`field`]), checked against the frozen reference by
//! both an Eval byte-identity test and an Emit textual-equivalence capture.
//!
//! # Approach — dual-instantiation, NO transpiler
//!
//! There is NO runtime AST and NO transpiler: the generic field body is ordinary
//! monomorphized Rust. The `f32` instantiation IS the machine code; the `Emit`
//! instantiation IS the codegen. This is the `T: FieldScalar` operator-overloading
//! eDSL pattern.
//!
//! # `no_std` + features
//!
//! The Eval path ([`scalar`] + [`field`]) is `#![no_std]`-clean (the physics leaf).
//! The one op stable `core` lacks is `sqrt`: the `nightly` feature uses
//! `core::intrinsics::sqrtf32` (strict `no_std`), else `std` is linked SOLELY for
//! `f32::sqrt` (mirroring `boyko_sdf_math`). The `emit` feature (OFF by default)
//! gates the std-side SSA recorder + the HLSL printer ([`emit`]) and the
//! `emit_field` bin, so a physics build NEVER links the emitter.
//!
//! IN-HOUSE: ZERO third-party deps. No rust-gpu / naga / spirv-builder.

// Strictly `#![no_std]` only when the `sqrt` intrinsic is available (the `nightly`
// feature) AND the std-side emitter is not requested. The `emit` feature pulls
// `std` (the SSA arena `Vec` + the `String` HLSL printer); without it the Eval
// path is `core`-only (default links `std` SOLELY for `f32::sqrt`).
#![cfg_attr(all(feature = "nightly", not(feature = "emit")), no_std)]
#![cfg_attr(feature = "nightly", feature(core_intrinsics))]
#![cfg_attr(feature = "nightly", allow(internal_features))]

pub mod brick;
pub mod field;
pub mod normal;
pub mod scalar;

#[cfg(feature = "emit")]
pub mod emit;

pub use brick::{cubic_eval, decode_snorm8, jcgt_cubic_coeffs, snorm_normalize, snorm_scale};
pub use field::{
    EditView, MAX_SDF_EDITS, SDF_FAR, combine, edit_distance, kind, op, sd_box, sd_sphere,
    sdf_field_body, smax, smin,
};
pub use normal::sdf_normal_body;
pub use scalar::FieldScalar;
