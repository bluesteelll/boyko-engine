//! Textured-PBR T6a: the host↔shader agreement gate for the `MATERIAL_FLAG_TEXTURED` bit.
//!
//! `boyko_rhi_vulkan::goldens` cannot depend on `boyko_render` (the dependency runs the other
//! way), so its `GOLDEN_MATERIAL_FLAG_TEXTURED` mirror is a SEPARATE literal from
//! `boyko_render::MATERIAL_FLAG_TEXTURED`. `boyko_render`'s `boyko_rhi_vulkan` dev-dependency
//! (`features = ["goldens"]`) is the ONE place both are simultaneously visible, so this crate is
//! the cross-check's natural home. The shader's `MATERIAL_FLAG_TEXTURED_BIT`
//! (`deferred_pbr.hlsl`, `#if !HWRT` block) is a THIRD, HLSL-side copy of the same literal —
//! unreachable from a Rust `const`/test (HLSL `static const` values are not SPIR-V-introspectable
//! by name), so its agreement is a source-comment convention, cross-referenced from all three
//! sites.

use boyko_render::material::MATERIAL_FLAG_TEXTURED;
use boyko_rhi_vulkan::goldens::GOLDEN_MATERIAL_FLAG_TEXTURED;

/// The three independent copies of the `MATERIAL_FLAG_TEXTURED` bit (host `MaterialGpu` writer,
/// host golden-oracle mirror, and — by source-comment convention — the HLSL resolve reader) MUST
/// agree, or a material's texture flag would be set by one side and silently misread by another.
#[test]
fn material_flag_textured_agrees_between_host_and_golden_mirror() {
    assert_eq!(
        MATERIAL_FLAG_TEXTURED, GOLDEN_MATERIAL_FLAG_TEXTURED,
        "boyko_render::MATERIAL_FLAG_TEXTURED must equal boyko_rhi_vulkan::goldens::\
         GOLDEN_MATERIAL_FLAG_TEXTURED (both must also equal deferred_pbr.hlsl's \
         MATERIAL_FLAG_TEXTURED_BIT — see that shader's binding-19 declaration)"
    );
}

/// Both mirrors are the single bit 1 — a `bitcast<f32>(flags) & BIT` HLSL test only works if the
/// value is a genuine single-bit mask (not e.g. accidentally `0` or a multi-bit value).
#[test]
fn material_flag_textured_is_bit_zero() {
    assert_eq!(MATERIAL_FLAG_TEXTURED, 1);
    assert_eq!(GOLDEN_MATERIAL_FLAG_TEXTURED, 1);
}
