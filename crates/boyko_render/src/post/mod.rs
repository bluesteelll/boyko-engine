//! Render wave PXe — the host side of the post chain's final pass, `post_final`.
//!
//! The shader is generated whole by `boyko_shaderdsl::emit::emit_post_final_comp` from the
//! single-source tables of `boyko_shaderdsl::post` and is NOT wired yet: PX1a creates its pipeline
//! and fills [`gpu::PostFinalGpu`] from the resolved post settings. This module holds only the
//! constant buffer's host mirror, pinned to the table the shader's `cbuffer` is printed from.

pub mod gpu;
