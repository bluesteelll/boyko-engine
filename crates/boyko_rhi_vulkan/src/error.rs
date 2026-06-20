//! The unified per-backend [`VulkanError`] (plan D4).
//!
//! The compute path's three rich error enums ([`BootError`],
//! [`MemoryError`], [`ComputeError`])
//! fold into ONE `VulkanError` used as the associated `Error` for every
//! operational trait ([`RhiDevice`](boyko_rhi::RhiDevice) /
//! [`RhiQueue`](boyko_rhi::RhiQueue) /
//! [`RhiCommandEncoder`](boyko_rhi::RhiCommandEncoder)). A single `Error` is what
//! an agnostic caller (`boyko_render`, the Phase-4 core seam) needs to `?`-chain
//! device + encoder + queue calls without per-trait fragments (critic W3(2)).
//!
//! The rich `command-name + VkResult` diagnostic the validation oracle relies on
//! is preserved verbatim in [`VulkanError::Vk`]. Two `From` directions exist:
//! the trait bound requires `From<RhiError>` (so seam stubs can build
//! `Err(RhiError::…​.into())`), and the agnostic projection
//! `From<VulkanError> for RhiError` maps the rich variants down to the
//! control-flow categories `boyko_render` branches on. Both are `#[cold]` /
//! `#[inline(never)]` so the `?`-desugar conversion never inlines into the hot
//! recording path's I-cache footprint (plan O4/W3(3)).
//!
//! There is deliberately **no** blanket `impl<E: Into<RhiError>> From<E> for
//! RhiError` — that is the reflexive-collision coherence wall (plan W3(1)). The
//! hand-written `From<VulkanError>` has no blanket, hence no collision.
//!
//! `swapchain.rs`'s `SwapchainError` is intentionally **not** folded in: the
//! on-screen path is untouched this phase and keeps its own error enum.

use boyko_rhi::RhiError;

use crate::compute::ComputeError;
use crate::device::BootError;
use crate::ffi::VkResult;
use crate::memory::MemoryError;

/// The unified Vulkan-backend error subsuming the compute path's boot / memory /
/// compute failures (plan D4).
///
/// Each variant keeps the diagnostic detail the original enum carried — most
/// importantly the `(command-name, VkResult)` pair the validation oracle uses to
/// pinpoint a fault. This is the single associated `Error` type bound on the
/// `RhiDevice` / `RhiQueue` / `RhiCommandEncoder` impls for the [`Vulkan`](crate::rhi_impl::Vulkan)
/// backend.
#[derive(Debug)]
pub enum VulkanError {
    /// A Vulkan command returned a non-success `VkResult`. The `&'static str`
    /// names the failing command (e.g. `"vkQueueSubmit"`).
    Vk(&'static str, VkResult),
    /// The Vulkan loader / instance / physical-device bootstrap failed. Carried
    /// whole so the loader-absent / GPU-absent categories survive (the tests'
    /// graceful-skip oracle reads them).
    Boot(BootError),
    /// No memory type satisfied the required property flags + type-bits mask.
    NoSuitableMemoryType,
    /// The host-visible block's sub-allocator could not satisfy a request
    /// (exhaustion).
    SubAllocExhausted,
    /// A declared-but-unimplemented RHI seam was invoked on this backend (plan
    /// D7). The `&'static str` names the method.
    Unsupported(&'static str),
    /// An agnostic [`RhiError`] carried verbatim (plan C3 / SEAM-4). Lets the
    /// `RhiError` → `VulkanError` → `RhiError` round-trip preserve the exact
    /// category instead of fabricating a `Vk(...)` for it. Constructed only by the
    /// `From<RhiError>` seam-stub path; projected back losslessly by
    /// `From<VulkanError> for RhiError`.
    Rhi(RhiError),
}

impl From<BootError> for VulkanError {
    #[cold]
    #[inline(never)]
    fn from(e: BootError) -> Self {
        VulkanError::Boot(e)
    }
}

impl From<MemoryError> for VulkanError {
    #[cold]
    #[inline(never)]
    fn from(e: MemoryError) -> Self {
        match e {
            MemoryError::NoSuitableMemoryType => VulkanError::NoSuitableMemoryType,
            MemoryError::VkError(cmd, result) => VulkanError::Vk(cmd, result),
            MemoryError::SubAllocExhausted => VulkanError::SubAllocExhausted,
        }
    }
}

impl From<ComputeError> for VulkanError {
    #[cold]
    #[inline(never)]
    fn from(e: ComputeError) -> Self {
        match e {
            ComputeError::VkError(cmd, result) => VulkanError::Vk(cmd, result),
            ComputeError::Memory(m) => VulkanError::from(m),
        }
    }
}

/// The reverse-direction conversion required by the trait bound
/// `Self::Error: From<RhiError>` (so a seam stub can write
/// `Err(RhiError::Unsupported("…").into())`).
///
/// Plan C3 (SEAM-4): the agnostic error is stored **verbatim** in
/// [`VulkanError::Rhi`] — no `Vk(...)` is fabricated. This makes the
/// `RhiError` → `VulkanError` → `RhiError` round-trip lossless (the back-projection
/// recovers the exact category), instead of mangling, say, `DeviceLost` into a
/// fake `Vk("RhiError::DeviceLost", …)`.
impl From<RhiError> for VulkanError {
    #[cold]
    #[inline(never)]
    fn from(e: RhiError) -> Self {
        VulkanError::Rhi(e)
    }
}

/// The agnostic projection (plan D4): collapse the rich backend variants onto the
/// control-flow categories an agnostic caller branches on. Hand-written (no
/// blanket → no coherence collision).
impl From<VulkanError> for RhiError {
    #[cold]
    #[inline(never)]
    fn from(e: VulkanError) -> Self {
        match e {
            VulkanError::Vk(_cmd, result) => match result {
                VkResult::ERROR_OUT_OF_HOST_MEMORY | VkResult::ERROR_OUT_OF_DEVICE_MEMORY => {
                    RhiError::OutOfMemory
                }
                VkResult::ERROR_OUT_OF_DATE_KHR => RhiError::SurfaceOutOfDate,
                VkResult::SUBOPTIMAL_KHR => RhiError::SuboptimalSurface,
                // Plan C2 (TD-4): a lost surface is a swapchain-recreate signal for
                // the Phase-2-3 on-screen caller, NOT a full device reboot — map it
                // to `SurfaceOutOfDate`, not `DeviceLost`.
                VkResult::ERROR_SURFACE_LOST_KHR => RhiError::SurfaceOutOfDate,
                // A non-success code that does not map to a finer category.
                _ => RhiError::BackendError("vulkan command failed"),
            },
            VulkanError::Boot(boot) => match boot {
                BootError::LoaderUnavailable
                | BootError::MissingCommand(_)
                | BootError::NoPhysicalDevice
                | BootError::NoSuitableQueueFamily
                | BootError::ValidationUnavailable
                | BootError::WindowingUnavailable
                | BootError::GbufferStorageFormatUnsupported
                | BootError::ViewtStorageFormatUnsupported => {
                    RhiError::BackendError("vulkan boot failed")
                }
                BootError::VkError(_cmd, result) => match result {
                    VkResult::ERROR_OUT_OF_HOST_MEMORY | VkResult::ERROR_OUT_OF_DEVICE_MEMORY => {
                        RhiError::OutOfMemory
                    }
                    _ => RhiError::BackendError("vulkan boot command failed"),
                },
            },
            VulkanError::NoSuitableMemoryType | VulkanError::SubAllocExhausted => {
                RhiError::OutOfMemory
            }
            VulkanError::Unsupported(method) => RhiError::Unsupported(method),
            // Plan C3: project a verbatim-carried agnostic error back losslessly.
            VulkanError::Rhi(rhi) => rhi,
        }
    }
}

impl core::fmt::Display for VulkanError {
    #[cold]
    #[inline(never)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VulkanError::Vk(cmd, result) => write!(f, "{cmd} failed: {result:?}"),
            VulkanError::Boot(e) => write!(f, "vulkan boot failed: {e:?}"),
            VulkanError::NoSuitableMemoryType => f.write_str("no suitable memory type"),
            VulkanError::SubAllocExhausted => f.write_str("sub-allocator exhausted"),
            VulkanError::Unsupported(method) => write!(f, "unsupported RHI operation: {method}"),
            VulkanError::Rhi(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for VulkanError {}
