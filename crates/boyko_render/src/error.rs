//! The `boyko_render` domain error type.
//!
//! Wraps the backend `VulkanError` (the only RHI error the Wave-B manager can
//! surface) plus the manager's own logic faults (a stale handle, a missing
//! mapped pointer on a staging buffer). Library code returns `Result<T,
//! GpuColumnError>` — never `anyhow`.

use boyko_rhi_vulkan::error::VulkanError;
use boyko_rhi_vulkan::swapchain::SwapchainError;

/// Errors raised by [`GpuColumnManager`](crate::GpuColumnManager) operations.
///
/// All variants are setup/teardown-path faults (allocation, upload, readback,
/// grow). The steady-state frame path ([`resolve`](crate::GpuColumnManager::resolve))
/// returns `Option`, not `Result` — a stale handle is `None`, not an error.
#[derive(Debug)]
pub enum GpuColumnError {
    /// A backend RHI call failed (buffer create, fence wait, submit, …).
    Rhi(VulkanError),
    /// A handle resolved to no live buffer in the registry — a grow bumped its
    /// generation (stale), or it was already destroyed. Loud per MF-7.
    StaleHandle,
    /// A host-visible staging buffer reported no mapped pointer, so the CPU
    /// cannot stage/read its bytes. A staging buffer is always host-visible, so
    /// this signals a backend invariant break, not a caller error.
    StagingNotMapped,
    /// A swapchain/present-path call failed (the per-frame in-flight fence wait the
    /// UI host driver issues before an upload — GUI P5a). Surfaced so the host driver
    /// can return ONE error type across the fence wait + the ring upload.
    Swapchain(SwapchainError),
    /// A deserialized asset violated the layout invariant its consumer relies on to
    /// size a GPU transfer — e.g. a `.bfont` whose `AtlasImage` claims a `w * h`
    /// extent that does not match `pixels.len()`. 2026-07 audit (CRITICAL): the
    /// extent drives the `vkCmdCopyBufferToImage` region while the staging buffer is
    /// sized from the payload, so a mismatch is an out-of-bounds DEVICE read. The
    /// invariant is enforced at the trust boundary (`read_bfont`) AND re-checked
    /// here, because a `BakedFont` can also be built in-process.
    MalformedAsset(&'static str),
}

impl From<SwapchainError> for GpuColumnError {
    #[inline]
    fn from(e: SwapchainError) -> Self {
        GpuColumnError::Swapchain(e)
    }
}

impl From<VulkanError> for GpuColumnError {
    #[inline]
    fn from(e: VulkanError) -> Self {
        GpuColumnError::Rhi(e)
    }
}

impl core::fmt::Display for GpuColumnError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GpuColumnError::Rhi(e) => write!(f, "RHI error: {e:?}"),
            GpuColumnError::StaleHandle => {
                write!(f, "device-column handle is stale (resolved to no live buffer)")
            }
            GpuColumnError::StagingNotMapped => {
                write!(f, "host-visible staging buffer has no mapped pointer")
            }
            GpuColumnError::Swapchain(e) => write!(f, "swapchain/present error: {e:?}"),
            GpuColumnError::MalformedAsset(what) => {
                write!(f, "malformed asset rejected before a GPU transfer: {what}")
            }
        }
    }
}

impl std::error::Error for GpuColumnError {}
