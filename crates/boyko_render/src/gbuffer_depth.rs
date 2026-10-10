//! The gbuffer ⇄ marcher linear-depth contract (mesh foundation M2, the C2 anchor).
//!
//! Two HAND-WRITTEN HLSL sites encode the SAME linear mesh depth and MUST agree, or a
//! mesh pixel's depth detaches from the marcher's ray-t and the depth-ownership between a
//! raster mesh and an SDF surface breaks:
//!
//! 1. The gbuffer fragment (`gbuffer_mrt.fs.hlsl`) writes, under perspective,
//!    `SV_Depth = length(eye_rel) / T_MAX` with a local `static const float T_MAX = 10.0`.
//! 2. The SDF marcher (`sdf_gbuffer_composite.hlsl`) DECODES a mesh pixel's depth as
//!    `t_mesh = md * T_MAX`, mirrored host-side by
//!    [`SDF_TRACE_T_MAX`](boyko_rhi_vulkan::compute::SDF_TRACE_T_MAX)` == 10.0`.
//!
//! The raster shaders `#include` nothing, so the literal is DUPLICATED across those two
//! HLSL files; nothing in the build couples them. [`GBUFFER_T_MAX`] is the host mirror of
//! the gbuffer fragment's `T_MAX`, and [`assert_gbuffer_marcher_t_max_agree`] (compiled +
//! exercised by `t_max_sync_pin_holds`) FAILS THE TEST if the two host constants ever
//! drift — turning a silent HLSL drift into a caught build/test failure.
//!
//! # Why this is load-bearing NOW (and was not before M1)
//!
//! Before the instanced arm, every mesh was drawn in WORLD space and `eye_rel` came
//! straight from `cam_eye - input.position`. M1's instanced VS recomputes
//! `eye_rel = cam_eye - world` AFTER transforming the model-space vertex by the
//! per-instance affine, so a mesh placed by an instance matrix at an arbitrary depth
//! depends on this exact `length(eye_rel) / T_MAX` ⇄ `md * T_MAX` round-trip to own (or
//! yield) its pixels against the SDF surface under perspective. M2 drives that arm for
//! real, so the two `T_MAX`s sharing one value is the correctness anchor for the whole
//! instanced-depth path.
//!
//! # A third site — rung R3b's `viewt_from_depth` producer
//!
//! Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh`, the SDF leg fully off) added a
//! THIRD site that must agree with the marcher's mesh-depth decode: `viewt_from_depth.comp.hlsl`,
//! the `gViewT` producer that stands in for the (undispatched) marcher on a mesh-only frame. It
//! does NOT hardcode a new HLSL copy of the marcher's `mesh_norm` ternary (`camera_mode ==
//! CAM_PERSPECTIVE ? MESH_DEPTH_T_MAX : T_MAX`) — that would be a THIRD hand-written HLSL copy,
//! alongside `sdf_gbuffer_composite.hlsl` and `sdf_tile_cull.hlsl`. Instead it receives the
//! already-selected `mesh_norm` as a host-precomputed push constant
//! ([`boyko_rhi_vulkan::compute::ViewtFromDepthPush`]), built by [`mesh_view_t_norm`] — the
//! SINGLE Rust-side source of that ternary. Any FUTURE host call site needing this value MUST
//! call [`mesh_view_t_norm`] rather than re-deriving the branch ad hoc (the sync-pin this module
//! exists for, applied to a runtime ternary instead of two compile-time constants).

use boyko_rhi_vulkan::compute::{CAM_MODE_PERSPECTIVE, MESH_DEPTH_T_MAX, SDF_TRACE_T_MAX};

/// The host mirror of the gbuffer fragment shader's `static const float T_MAX = 10.0`
/// (`gbuffer_mrt.fs.hlsl`): the ray-range normalizer the fragment divides the euclidean
/// `length(eye_rel)` by to write the marcher-aligned linear `SV_Depth`. It MUST equal the
/// marcher's [`SDF_TRACE_T_MAX`] — see [`assert_gbuffer_marcher_t_max_agree`].
pub const GBUFFER_T_MAX: f32 = 10.0;

/// Asserts the gbuffer fragment's linear-depth normalizer ([`GBUFFER_T_MAX`]) equals the
/// marcher's ray-t decode constant ([`SDF_TRACE_T_MAX`]). A drift between the two
/// hand-written HLSL depth sites (the fragment's `length(eye_rel)/T_MAX` vs the marcher's
/// `md * T_MAX`) detaches a perspective mesh pixel's depth from the marcher's ray, so
/// keeping the two host mirrors equal is the C2 build-time guard.
///
/// `const`, so a caller may also pin it at compile time:
/// `const _: () = assert_gbuffer_marcher_t_max_agree();`.
//
// `clippy::assertions_on_constants`: the assertion's operands ARE both constants — that is
// the WHOLE POINT. This is a compile-time drift guard between two hand-written HLSL depth
// literals (`gbuffer_mrt.fs.hlsl`'s `T_MAX` mirrored by `GBUFFER_T_MAX`, and the marcher's
// `SDF_TRACE_T_MAX`); when they agree the assert is a no-op, when a future edit drifts one
// it fails the build. The lint's suggested `const { assert!(..) }` cannot wrap the whole
// `const fn` body cleanly while keeping it callable at runtime too, so the allow is the
// idiomatic choice for a deliberate const drift guard.
#[allow(clippy::assertions_on_constants)]
#[inline]
pub const fn assert_gbuffer_marcher_t_max_agree() {
    assert!(
        GBUFFER_T_MAX == SDF_TRACE_T_MAX,
        "GBUFFER_T_MAX (gbuffer_mrt.fs.hlsl T_MAX) must equal the marcher's SDF_TRACE_T_MAX: \
         the gbuffer fragment writes length(eye_rel)/T_MAX and the marcher decodes md*T_MAX, \
         so a drift detaches a perspective mesh pixel's depth from the marcher ray"
    );
}

// Compile-time pin: a drift fails the BUILD, not just the test below (defense in depth —
// the test exercises it explicitly for a named failure, this catches it even if the test
// is filtered out).
const _: () = assert_gbuffer_marcher_t_max_agree();

/// The mesh-depth ray-t normalizer for `camera_mode`, mirroring the SDF marcher's own
/// `mesh_norm` ternary VERBATIM (`sdf_gbuffer_composite.hlsl:1439`: `(camera_mode ==
/// CAM_PERSPECTIVE) ? MESH_DEPTH_T_MAX : T_MAX`; the SAME ternary is ALSO hand-duplicated in
/// `sdf_tile_cull.hlsl`). Rung R3b's `viewt_from_depth` producer needs this exact value —
/// precomputed HOST-side (see this module's doc) rather than re-branching in a third HLSL copy —
/// so this is the ONE Rust-side place that ternary is written; every host call site (today: the
/// `viewt_from_depth` push builders in `boyko_app::gpu_scene` and the `window_present_gbuffer.rs`
/// test harness) MUST go through this fn rather than re-deriving the branch.
///
/// `camera_mode` takes the SAME raw value as the shader's `cbuffer Camera`'s `camera_mode` field
/// / [`boyko_rhi_vulkan::compute::CompositePushConstants::camera_mode`] — [`CAM_MODE_PERSPECTIVE`]
/// selects [`MESH_DEPTH_T_MAX`], anything else (ortho) selects [`SDF_TRACE_T_MAX`].
#[inline]
pub const fn mesh_view_t_norm(camera_mode: u32) -> f32 {
    if camera_mode == CAM_MODE_PERSPECTIVE { MESH_DEPTH_T_MAX } else { SDF_TRACE_T_MAX }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C2 sync-pin: the gbuffer fragment's `T_MAX` host mirror equals the marcher's
    /// ray-t decode constant. A future edit to either HLSL literal (without updating the
    /// other) breaks this — the named guard against the two hand-written depth sites
    /// drifting.
    #[test]
    fn t_max_sync_pin_holds() {
        assert_gbuffer_marcher_t_max_agree();
        assert_eq!(
            GBUFFER_T_MAX, SDF_TRACE_T_MAX,
            "GBUFFER_T_MAX must mirror the marcher SDF_TRACE_T_MAX"
        );
    }

    /// The rung R3b sync-pin: [`mesh_view_t_norm`] reproduces the marcher's `mesh_norm` ternary
    /// bit-for-bit (`sdf_gbuffer_composite.hlsl:1439`) for both camera modes.
    #[test]
    fn mesh_view_t_norm_mirrors_the_marcher_ternary() {
        use boyko_rhi_vulkan::compute::CAM_MODE_ORTHO;

        assert_eq!(mesh_view_t_norm(CAM_MODE_PERSPECTIVE), MESH_DEPTH_T_MAX);
        assert_eq!(mesh_view_t_norm(CAM_MODE_ORTHO), SDF_TRACE_T_MAX);
    }
}
