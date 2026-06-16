//! The [`RhiQueue`] operational trait: command submission.
//!
//! Phase 1 is headless, so submission takes no semaphores — the only sync is the
//! signaled `fence`, waited via [`crate::device::RhiDevice::wait_fence`]. The
//! semaphore-waited present submit (`submit_windowed`) is a Phase-2-3 seam and is
//! intentionally absent from the Phase-1 surface.

use crate::api::RhiApi;
use crate::error::RhiError;

/// The submission queue.
///
/// A backend implements this on a thin queue wrapper (Vulkan: `VulkanQueue`,
/// matching wgpu-hal + DX12; plan O1).
pub trait RhiQueue<A: RhiApi> {
    /// One unified per-backend error type (plan D4); bound is `From<RhiError>`
    /// only — see [`crate::device::RhiDevice::Error`].
    type Error: core::fmt::Debug + From<RhiError>;

    /// Submits one recorded `encoder`, signaling `signal_fence` on completion.
    ///
    /// No semaphores: the headless path's only sync point is the fence.
    ///
    /// # Lifetime contract (plan F1 / RL-1)
    ///
    /// The originating device/context (the one this queue, `encoder` and
    /// `signal_fence` came from) MUST still be alive — submitting after it is
    /// dropped is **undefined behavior** (backend resources hold raw pointers into
    /// the context). No compile-time `'ctx` tie this phase; the structural fix is
    /// deferred to Phase 2-3.
    fn submit(
        &self,
        encoder: &A::CommandEncoder,
        signal_fence: &A::Fence,
    ) -> Result<(), Self::Error>;
}
