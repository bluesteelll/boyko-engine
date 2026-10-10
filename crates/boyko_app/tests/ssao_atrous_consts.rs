//! Cross-crate const-equality guard: `boyko_render::MAX_SSAO_ATROUS_LEVELS` and
//! `boyko_rhi_vulkan::present::MAX_SSAO_ATROUS_LEVELS` must stay equal. The RHI cannot depend on
//! `boyko_render` (the render crate sits ABOVE it), so the SSAO à-trous level-count ceiling is
//! duplicated in both crates; `boyko_app` links both, so it is the single point that can assert
//! the two never drift apart (mirrors the existing `MAX_ATROUS_LEVELS` duplication for the
//! shadow-visibility denoiser, which has no equivalent guard test — this is new coverage).

#[test]
fn max_ssao_atrous_levels_consts_agree() {
    assert_eq!(
        boyko_render::MAX_SSAO_ATROUS_LEVELS,
        boyko_rhi_vulkan::present::MAX_SSAO_ATROUS_LEVELS,
        "boyko_render::MAX_SSAO_ATROUS_LEVELS must equal boyko_rhi_vulkan::present::MAX_SSAO_ATROUS_LEVELS"
    );
}
