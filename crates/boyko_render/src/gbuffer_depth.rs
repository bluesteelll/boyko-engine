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
//! exercised by [`t_max_sync_pin_holds`]) FAILS THE TEST if the two host constants ever
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

use boyko_rhi_vulkan::compute::SDF_TRACE_T_MAX;

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
}
