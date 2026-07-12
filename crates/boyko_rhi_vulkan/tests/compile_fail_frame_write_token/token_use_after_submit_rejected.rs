//! R0b — a `FrameWriteToken` consumed BY VALUE by the frame-ending
//! `Renderer::render_gbuffer_frame` must NOT be usable afterwards.
//!
//! The submit ends the frame's host-write window: a write proof surviving it
//! would reopen the write-after-read hazard on the per-slot rings (the slot's
//! N−2 occupant race the token exists to close). With `Clone`/`Copy` removed
//! (R0b) the post-submit use is a move error.
//!
//! Every handle is passed in by parameter (never constructed), so the failure
//! is isolated to the use-after-move, not a construction error.

use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::VkExtent2D;
use boyko_rhi_vulkan::swapchain::{
    FrameWriteToken, GBufferFrame, GBufferScene, Renderer, Surface, Swapchain,
};

/// Consumes `token` in the submit, then reads its slot — rejected (use after move).
unsafe fn write_after_submit<'ctx>(
    renderer: &mut Renderer<'ctx>,
    token: FrameWriteToken,
    ctx: &VulkanContext,
    surface: &Surface<'_>,
    swapchain: &mut Swapchain<'ctx>,
    scene: &GBufferScene<'_>,
    frame: &mut GBufferFrame,
    extent: VkExtent2D,
) -> usize {
    // SAFETY: never executed — a compile-fail case; the call only needs to typecheck.
    let _ = unsafe {
        renderer.render_gbuffer_frame(
            token, ctx, surface, swapchain, scene, frame, 64, 64, [0.0; 4], extent, extent, None,
        )
    };
    token.slot()
}

fn main() {}
