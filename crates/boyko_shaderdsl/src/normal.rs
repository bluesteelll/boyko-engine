//! The SDF surface-normal leaf, authored ONCE generic over [`FieldScalar`].
//!
//! [`sdf_normal_body`] is the central-difference gradient of the WHOLE edit-list
//! field, mirroring the frozen reference operand-for-operand:
//! `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli:193-200` (the GPU normal) AND
//! `boyko_sdf_math::sdf_edit_list_normal` (lib.rs:696-704, the CPU normal).
//! Instantiating with `S = f32` ([`crate::scalar`]'s Eval impl) reproduces the
//! hand-written CPU normal BYTE-IDENTICALLY; instantiating with `S = Emit`
//! ([`crate::emit`]) records the op tree the HLSL printer walks into `sdf_normal`.
//!
//! # The field-call seam
//!
//! Unlike the polynomial leaves ([`crate::field`]), the normal does NOT inline the
//! field: it CALLS `sdf` at six probe points (`p ± offset` on each axis). The frozen
//! HLSL keeps `sdf` a separate hand-written function (it owns the `[loop]` over the
//! edit list), so the normal must record CALLS to `sdf`, not unroll it. The seam is a
//! generic field callback `field: F` (`F: Fn(S::Vec3) -> S`, NO `Box`/`dyn`):
//! - On **Eval** the test passes `|q| sdf_edit_list(edits, q)`, so the body literally
//!   re-runs the host field at each probe — making `sdf_normal_body::<f32>` reproduce
//!   `sdf_edit_list_normal` exactly.
//! - On **Emit** the callback records a `Call` node the printer prints as `sdf(<arg>)`.
//!
//! # `no_std`
//!
//! `#![no_std]`-clean (the Eval path is a leaf, like [`crate::field`]). The callback
//! is a monomorphized `Fn` value — no allocation, no dynamic dispatch.

use crate::scalar::{self, FieldScalar};

/// Surface normal via central differences of the field `field` at `p` — the
/// gradient of the WHOLE edit-list field, normalized.
///
/// Replicates `boyko_sdf_math::sdf_edit_list_normal` (lib.rs:696-704) and the frozen
/// `sdf_normal` (`sdf_field.hlsli:193-200`) operand-for-operand:
///
/// ```text
/// e = (GRAD_H, 0.0)
/// n = ( sdf(p + e.xyy) - sdf(p - e.xyy),
///       sdf(p + e.yxy) - sdf(p - e.yxy),
///       sdf(p + e.yyx) - sdf(p - e.yyx) )
/// return normalize(n)
/// ```
///
/// `field` is the field-call seam (see the module doc): on Eval it is the host
/// `sdf_edit_list` closure; on Emit it records a `sdf(...)` call node. The op order
/// is load-bearing — a reordering would shift a committed GPU golden past its
/// `±2/255` tolerance.
#[inline]
pub fn sdf_normal_body<S: FieldScalar<Vec3 = [S; 3]>, F: Fn([S; 3]) -> S>(
    p: [S; 3],
    field: F,
) -> [S; 3] {
    // e.xyy / e.yxy / e.yyx — the GRAD_H axis offsets (Eval: literal `[h,0,0]`...;
    // Emit: the three textual swizzles off `float2 e = float2(GRAD_H, 0.0)`).
    let [ox, oy, oz] = S::grad_offsets();
    // Per axis: sdf(p + offset) - sdf(p - offset). `v_add`/`v_sub` form the probe
    // points; the field callback evaluates `sdf` at each (the field-call seam).
    let nx = field(scalar::v_add(p, ox)).sub(field(scalar::v_sub(p, ox)));
    let ny = field(scalar::v_add(p, oy)).sub(field(scalar::v_sub(p, oy)));
    let nz = field(scalar::v_add(p, oz)).sub(field(scalar::v_sub(p, oz)));
    // `normalize(n)` — RAW on Emit (HLSL `normalize`), GUARDED on Eval (byte-mirrors
    // `boyko_sdf_math::v_normalize`: a zero/non-finite length collapses to ZERO).
    S::v_normalize([nx, ny, nz])
}
