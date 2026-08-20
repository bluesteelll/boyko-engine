//! `boyko_shaderdsl` — the in-house Rust shader eDSL (Pass 1: CPU-verifiable).
//!
//! Author the SDF field math ONCE, generic over a [`FieldScalar`](scalar::FieldScalar)
//! backend, and instantiate it two ways:
//!
//! - `S = f32` — the **Eval** backend ([`scalar`]): every op is a single `core`
//!   f32 instruction, BYTE-IDENTICAL to the hand-written `boyko_sdf_math` field
//!   (`boyko_sdf_math` DELEGATES to [`field`] over `f32`).
//! - `S = Emit` — the **HLSL SSA recorder** (`emit`, `feature = "emit"`): each op
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
//! gates the std-side SSA recorder + the HLSL printer (`emit`) and the
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
pub mod cf;
pub mod cubic_hit;
pub mod decl;
pub mod field;
pub mod half;
pub mod levels;
pub mod marcher;
pub mod normal;
pub mod oct;
pub mod pack;
pub mod particle_facets;
pub mod probe_blend;
pub mod probe_march;
pub mod refine;
pub mod remarch;
pub mod scalar;
pub mod shadow;
pub mod sor;
pub mod ssao;
pub mod surface;
pub mod vb;

#[cfg(feature = "emit")]
pub mod emit;

// Pillar B increment B2 — the TRS interpolation math. Codegen-only (it needs std trig
// for its Eval oracle and records into the `Emit` HLSL recorder), so it is gated behind
// `emit` exactly like the recorder itself: a physics `no_std` build never links it.
#[cfg(feature = "emit")]
pub mod interp;

pub use brick::{
    BRICK_OUTSIDE_GRID, brick_cell_class_body, cubic_eval, decode_snorm8, jcgt_cubic_coeffs,
    snorm_normalize, snorm_scale,
};
pub use cf::{Cf, EvalCf, Flow, LoopOp};
pub use cubic_hit::{
    M2_APRON, M2_ATLAS_BIAS, M2_MAX_CELLS as M2_CUBIC_MAX_CELLS, m2_brick_cubic_hit_body,
};
pub use decl::{b1_decl_exhausted_body, b1_decl_hit_body};
pub use field::{
    EditView, MAX_SDF_EDITS, SDF_FAR, combine, edit_distance, kind, op, sd_box, sd_sphere,
    sdf_field_body, smax, smin,
};
pub use half::{f16_bits_to_f32, f32_to_f16_bits};
pub use levels::{BRICK_LEVELS, select_level_body};
pub use marcher::{b1_marcher_fold_d_body, b1_marcher_mesh_p_body};
pub use normal::sdf_normal_body;
pub use oct::{oct_decode_body, oct_encode_body};
pub use pack::pack_material_id_ba_body;
pub use particle_facets::{
    e1_bit_mix_body, e2_bitcast_sign_flip_body, e2_pack_half2_body, e2_unpack_half2_body,
    e3_dot_body, e4_trig_body, e5_renorm_body,
};
pub use probe_blend::{probe_blend_body, probe_depth_blend_body};
pub use probe_march::{
    GI_HIT_EPS, GI_MAX_IT, GI_MINT, GI_MINT_STEP, GI_T_MAX, probe_march_body,
};
pub use refine::{
    EPS as B1_REFINE_EPS, M2_REFINE_ITERS as B1_REFINE_ITERS, M2_REFINE_RELAX as B1_REFINE_RELAX,
    b1_accept_refine_body,
};
pub use remarch::{
    EPS as B1_REMARCH_EPS, MAX_IT as B1_REMARCH_MAX_IT, T_MAX as B1_REMARCH_T_MAX,
    b1_exhaustion_remarch_body,
};
#[cfg(feature = "emit")]
pub use interp::{InterpBackend, SLERP_DOT_THRESHOLD, slerp_quat, transform_pair_interp_body};
pub use scalar::FieldScalar;
pub use shadow::{
    FIELD_LIPSCHITZ_L, MAX_IT, SHADOW_HIT_EPS, SHADOW_K, SHADOW_MINT, SHADOW_MINT_STEP, T_MAX,
    sdf_soft_shadow_body,
};
pub use sor::{
    FIELD_LIPSCHITZ_L as B1_SOR_LIPSCHITZ_L, T_MAX as B1_SOR_T_MAX, b1_sor_retreat_body,
};
pub use ssao::{
    SSAO_EPS, SSAO_RADIUS, SSAO_SLICES, SSAO_STEPS, SSAO_STRENGTH, ssao_estimate_body,
    ssao_horizon_step_body, ssao_slice_body,
};
pub use surface::{
    EPS as M2_SURFACE_EPS, M2_REFINE_ITERS, M2_REFINE_RELAX, T_MAX as M2_SURFACE_T_MAX,
    m2_surface_hit_refine_body,
};
pub use vb::{
    BaryBasis, NEAR_CLIP_DENOM_EPS, NEAR_CLIP_W_EPSILON, vb_barycentric_eval_body,
    vb_barycentric_grad_body, vb_interp_body, vb_near_clip_body, vb_uv_grad_body,
};
