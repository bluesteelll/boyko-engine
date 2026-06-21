//! `boyko_math` — the single SIMD-aligned POD math vocabulary for the engine.
//!
//! All types are `#[repr(C)]` plain-old-data (`Copy`, no `Drop`, no interior
//! pointers), SIMD-aligned where it matters, with a full operator/method set.
//!
//! # Bit-determinism (INVIOLABLE)
//!
//! The engine runs **bit-deterministic** physics. [`Vec3`], [`Quat`], and
//! [`Mat3`] are lifted **verbatim** (algorithm- and instruction-identical) from
//! the physics foundation, so the migrated physics is bit-for-bit unchanged:
//!
//! - Normalization is literally `len_sq.sqrt().recip()` — **exact `sqrt`** then
//!   reciprocal, NOT a hardware `rsqrt`.
//! - There is **no** `f32::mul_add`/FMA, no `*_rsqrt`/`*_rcp` approximation, no
//!   `fast-math`/`float_algebraic` anywhere in this crate. New ops added for the
//!   new types (`Vec2`/`Vec4`/`Mat4`/[`Affine3A`]) follow the same discipline:
//!   every multiply-then-add is written as separate statements so the default
//!   Rust codegen does NOT contract them into an FMA.
//! - Signed-zero / `NaN` tie behavior of `abs`/`clamp` matches the lifted code.
//!
//! # Matrix conventions
//!
//! - [`Mat3`] is **row-major** (`rows[i]` is row `i`) — the lifted physics
//!   convention, kept exactly.
//! - [`Mat4`] is **column-major** (`cols[i]` is column `i`) — the WGSL `mat4x4`
//!   convention, so it uploads directly to a GPU uniform.
//! - [`Affine3A`]'s linear part is the **row-major** [`Mat3`] and reuses its ops
//!   verbatim.
//! - The **only** place the row-major ↔ column-major boundary is crossed is
//!   [`Affine3A::to_mat4`] / [`Mat4::from_affine`].

pub mod affine;
pub mod mat;
pub mod quat;
pub mod vec;

pub use affine::Affine3A;
pub use mat::{Mat3, Mat4};
pub use quat::Quat;
pub use vec::{Vec2, Vec3, Vec4};
