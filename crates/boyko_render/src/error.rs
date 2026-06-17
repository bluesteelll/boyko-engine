//! The `boyko_render` domain error type.
//!
//! Wraps the backend `VulkanError` (the only RHI error the Wave-B manager can
//! surface) plus the manager's own logic faults (a stale handle, a missing
//! mapped pointer on a staging buffer). Library code returns `Result<T,
//! GpuColumnError>` — never `anyhow`.

use boyko_rhi_vulkan::error::VulkanError;

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
        }
    }
}

impl std::error::Error for GpuColumnError {}
