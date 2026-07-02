//! R0b — `FrameWriteToken` must NOT be clonable.
//!
//! A cloned write proof would survive the frame-ending by-value consume
//! (`render_gbuffer_frame` / `present_sampled`) or be stashed across frames —
//! either way the affine fence discipline is void. The token derives only
//! `Debug`, so `.clone()` resolves to no method.

use boyko_rhi_vulkan::swapchain::FrameWriteToken;

/// Duplicates the write proof — rejected (no `Clone` impl).
fn duplicate(token: FrameWriteToken) -> (FrameWriteToken, FrameWriteToken) {
    let copy = token.clone();
    (token, copy)
}

fn main() {}
