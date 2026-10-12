//! The **SDF depth-merge encode** (render wave RD1, defect R0b): the eDSL leaves that turn a
//! composite ray parameter `t` (the `gViewT` lane) back into the path's own depth encode, so a
//! full-screen `SV_Depth` pass can write SDF surfaces into the depth image every depth-tested
//! transparent consumer (particles today, translucent meshes from R1) tests against.
//!
//! # The encode is the INVERSE of each marcher's decode
//!
//! The merge does not project `P = eye + rd·t` through a view-projection matrix: the camera block
//! the marchers read carries no projection rows. It inverts the decode each path's marcher already
//! applies to its depth image when it builds the composite `t`:
//!
//! - **Deferred** (custom-linear depth, `LESS`): the composite decodes `t_mesh = md · mesh_norm`
//!   with `mesh_norm = MESH_DEPTH_T_MAX` (64) under a perspective camera and `T_MAX` (10) under
//!   the ortho one (`sdf_gbuffer_composite.hlsl`). The inverse is `t / mesh_norm` —
//!   [`sdf_t_to_depth_linear_body`].
//! - **Forward / ForwardPlus / VisibilityBuffer** (hardware reverse-Z, `GREATER`): the marcher
//!   decodes `view_z = B / (d − A)`, `t = view_z / max(dot(fwd, rd), 1e-4)`
//!   (`sdf_forward_march.comp.hlsl`, `boyko_render::view::forward_view_z_coeffs`). The inverse is
//!   `A + B / (t · max(dot(fwd, rd), 1e-4))` — [`sdf_t_to_depth_reverse_z_body`]. The reverse-Z
//!   projection is perspective-only, so there is no reverse-Z ortho arm.
//!
//! # The bias is the discriminator
//!
//! Both leaves encode `t · (1 + SDF_DEPTH_MERGE_BIAS)`, `β = 2⁻¹⁶`. At a pixel a mesh won, the
//! composite `t` IS the decoded mesh `t`, so the biased encode lands strictly BEHIND the stored
//! mesh depth and the path's depth test rejects the write — the mesh depth is untouched. At a
//! pixel the SDF won by more than `β` (relative), the biased encode is still in front of whatever
//! is stored, so it writes. The margin (≈ 256 f32 ULPs of the reconstructed view depth) dwarfs
//! the round-trip error of the decode/encode pair (a handful of roundings), which is what the
//! host discriminator oracle in `boyko_rhi_vulkan/tests/sdf_depth_merge_edsl_sync.rs` measures.
//!
//! `1 + β` is exactly representable in `f32` (17 significant bits), so `t · (1.0 + BIAS)` is one
//! rounding on both backends: the Eval product and DXC's constant-folded HLSL product agree.
//!
//! # Instantiation
//!
//! - `<EvalCf>` — the CPU oracle, swept against an `f64` reference by the sync test.
//! - `<EmitCf>` — the HLSL recorder (`crate::emit::emit_hlsl_sdf_t_to_depth_*`); the spans are
//!   spliced into `boyko_rhi_vulkan/shaders/sdf_depth_merge.fs.hlsl`, which the
//!   `emit_depth_merge` bin owns whole.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The denominator of [`SDF_DEPTH_MERGE_BIAS`] — `β = 1 / 65536 = 2⁻¹⁶`. The generator prints the
/// HLSL constant as `1.0 / 65536.0` from this number, an expression that is exact in both `f32`
/// and `f64`, so the shader and the host leaf carry one value with no decimal re-spelling.
pub const SDF_DEPTH_MERGE_BIAS_DENOM: u32 = 65536;

/// The relative depth-merge bias `β = 2⁻¹⁶`, spelled as the named literal `SDF_DEPTH_MERGE_BIAS`
/// in the generated HLSL. See the module doc for why it is the merge's discriminator.
pub const SDF_DEPTH_MERGE_BIAS: f32 = 1.0 / SDF_DEPTH_MERGE_BIAS_DENOM as f32;

/// The merge push's `mode` word for the Deferred custom-linear encode
/// ([`sdf_t_to_depth_linear_body`]). A generator input: the shader spells it as a named constant
/// and the host push (`boyko_rhi_vulkan::compute::SdfDepthMergePush`) mirrors it, pinned by the
/// sync test.
pub const SDF_DEPTH_MERGE_MODE_LINEAR: u32 = 0;
/// The merge push's `mode` word for the reverse-Z encode ([`sdf_t_to_depth_reverse_z_body`]).
pub const SDF_DEPTH_MERGE_MODE_REVERSE_Z: u32 = 1;

const _: () = assert!(SDF_DEPTH_MERGE_BIAS_DENOM.is_power_of_two());
// `1 + β` must be an exact f32 so the biased product is a single rounding on both backends.
const _: () = assert!(SDF_DEPTH_MERGE_BIAS_DENOM <= 1 << 23);

/// The literal `1.0` the bias is added to.
const ONE: f32 = 1.0;

/// `t · (1.0 + SDF_DEPTH_MERGE_BIAS)` — the biased ray parameter both leaves encode.
#[inline]
fn biased_t<C: Cf>(t: C::Scalar) -> C::Scalar {
    t.mul(C::Scalar::lit(ONE).add(C::named_lit("SDF_DEPTH_MERGE_BIAS", SDF_DEPTH_MERGE_BIAS)))
}

/// **Deferred** — the custom-linear encode: `(t · (1 + β)) / norm`, where `norm` is the marcher's
/// own decode normalizer (`mesh_view_t_norm(camera_mode)`: 64 perspective, 10 ortho). Deposits the
/// depth into `ret_out`.
///
/// ```text
/// float t_b = t * (1.0 + SDF_DEPTH_MERGE_BIAS);
/// return t_b / norm;
/// ```
#[inline]
pub fn sdf_t_to_depth_linear_body<C: Cf>(
    t: C::Scalar,
    norm: C::Scalar,
    ret_out: &C::RetCellF,
) -> Flow {
    let t_b = C::temp_float("t_b", biased_t::<C>(t));
    C::ret_f(ret_out, t_b.div(norm))
}

/// **Reverse-Z** (Forward / ForwardPlus / VisibilityBuffer) — `A + B / (t · (1 + β) · cos_fwd)`,
/// where `cos_fwd = max(dot(cam_forward, rd), 1e-4)` is the marcher's own ray-to-view-depth factor
/// and `(a, b)` are `forward_view_z_coeffs(near, far)`. Deposits the depth into `ret_out`.
///
/// ```text
/// float view_z = t * (1.0 + SDF_DEPTH_MERGE_BIAS) * cos_fwd;
/// return a + b / view_z;
/// ```
#[inline]
pub fn sdf_t_to_depth_reverse_z_body<C: Cf>(
    t: C::Scalar,
    cos_fwd: C::Scalar,
    a: C::Scalar,
    b: C::Scalar,
    ret_out: &C::RetCellF,
) -> Flow {
    let view_z = C::temp_float("view_z", biased_t::<C>(t).mul(cos_fwd));
    C::ret_f(ret_out, a.add(b.div(view_z)))
}

/// Host convenience: [`sdf_t_to_depth_linear_body`] over the `f32` Eval backend.
#[inline]
pub fn sdf_t_to_depth_linear(t: f32, norm: f32) -> f32 {
    let cell = core::cell::Cell::new(0.0f32);
    let _ = sdf_t_to_depth_linear_body::<crate::cf::EvalCf>(t, norm, &cell);
    cell.get()
}

/// Host convenience: [`sdf_t_to_depth_reverse_z_body`] over the `f32` Eval backend.
#[inline]
pub fn sdf_t_to_depth_reverse_z(t: f32, cos_fwd: f32, a: f32, b: f32) -> f32 {
    let cell = core::cell::Cell::new(0.0f32);
    let _ = sdf_t_to_depth_reverse_z_body::<crate::cf::EvalCf>(t, cos_fwd, a, b, &cell);
    cell.get()
}
