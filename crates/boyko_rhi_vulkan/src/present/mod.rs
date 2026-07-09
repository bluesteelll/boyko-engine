//! Slice-1 — Vulkan surface + swapchain + present, rendering a cleared frame via
//! **Vulkan 1.3 dynamic rendering** (no `VkRenderPass` / `VkFramebuffer`).
//!
//! Per `docs/RENDER-PHYSICS-GPU-PLAN.md` §7 (Phase 1-3 on-screen path), this
//! completes the on-screen seam over the raw Win32 [`crate::window::Window`] and
//! the windowed [`crate::device::VulkanContext`]:
//!
//! - [`Surface`] wraps `vkCreateWin32SurfaceKHR` over an `HWND`/`HINSTANCE`,
//!   confirms a present-capable queue family via
//!   `vkGetPhysicalDeviceSurfaceSupportKHR`, and selects a present-capable color
//!   format (preferring `B8G8R8A8_UNORM` / `_SRGB`).
//! - [`Swapchain`] queries surface caps/formats/present-modes, creates a
//!   FIFO-present-mode `VkSwapchainKHR` with `COLOR_ATTACHMENT` images, fetches
//!   the images and a `VkImageView` per image, and recreates itself on resize /
//!   `VK_ERROR_OUT_OF_DATE_KHR` / `VK_SUBOPTIMAL_KHR`.
//! - [`Renderer`] owns the per-frame sync (2 frames in flight) and runs the
//!   acquire → record (barrier → `vkCmdBeginRendering` clear → `vkCmdEndRendering`
//!   → barrier) → submit → present loop, recreating the swapchain when needed.
//!
//! # Soundness oracle (raw FFI → no Miri)
//!
//! Raw driver FFI cannot run under Miri; the oracle (plan §6) is the
//! `VK_LAYER_KHRONOS_validation` messenger asserted to `total() == 0` plus clean
//! reverse-order teardown (no leaked-object validation reports). Every `unsafe`
//! states the invariant that makes it sound (sync ordering, barrier params,
//! handle lifetimes, fence-before-destroy).
//!
//! # Teardown order
//!
//! Reverse of creation, device-idle first: per-frame sync → image views →
//! swapchain → surface → (window, destroyed by the caller after). Each owner's
//! `Drop` handles its own objects; the caller drops `Renderer` → `Swapchain` →
//! `Surface` before the [`crate::window::Window`].

use crate::ffi::{
    VK_IMAGE_ASPECT_COLOR_BIT, VK_IMAGE_ASPECT_DEPTH_BIT, VkImageSubresourceRange, VkResult,
};

mod frame_driver;
pub mod gpu_timing;
mod graph_bridge;
mod passes;
mod scene_types;
mod surface;
mod swapchain;
mod targets;

pub use frame_driver::{FrameWriteToken, Renderer};
pub use gpu_timing::{PASS_COUNT, TimedPass, TimestampCollector};
pub use scene_types::{
    BrickActivation, CsmDepthActivation, DdgiUpdateActivation, GBUFFER_IDENTITY_INSTANCE,
    GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES, GBufferMeshDraw, GBufferScene,
    InterpActivation, PunctualDepthActivation, SCENE_MVP_BYTES, SampledComposite, Scene,
    SsaoActivation, UiPass,
};
#[cfg(feature = "hwrt")]
pub use scene_types::{ShadowVisActivation, TlasBuildActivation};
pub use surface::Surface;
pub use swapchain::Swapchain;
pub use targets::{GBufferFrame, GBufferTargets};

/// The number of frames the [`Renderer`] keeps in flight (double-buffered CPU↔GPU
/// overlap). Per-frame: an acquire semaphore + an in-flight fence; render-finished
/// semaphores are per swapchain image (so a present is never signalled by a
/// semaphore still pending another image's present).
///
/// Exported so a host can size its per-frame UBO RING (one slot per in-flight frame)
/// to match the renderer's round-robin [`Swapchain::frame_index`] — the lock-free
/// write-after-read fix: each frame writes `ring[frame_index]` and the GPU binds that
/// same slot, so the sibling in-flight frame reads a DIFFERENT slot (no overlap).
pub const FRAMES_IN_FLIGHT: usize = 2;

/// HW-RT rung 3a: the max à-trous spatial-denoise iterations the recorder can dispatch (the
/// ping-pong / pass-array bound). This is the RHI-layer MIRROR of `boyko_render`'s authoritative
/// `MAX_ATROUS_LEVELS` — the RHI cannot depend on `boyko_render` (the render crate sits ABOVE it),
/// so the value is duplicated here and the host (which links both) is the single point that keeps
/// them in lock-step: `boyko_render::ShadowDenoiseConfig::clamped_levels` clamps to
/// `boyko_render::MAX_ATROUS_LEVELS`, and a `ShadowVisActivation.levels` beyond THIS const would
/// index past the per-level `GbufferPassPlan.shadow_atrous` / `atrous_set` arrays (a debug-asserted
/// invariant at the record site). Kept equal to `boyko_render::MAX_ATROUS_LEVELS` (5).
#[cfg(feature = "hwrt")]
pub const MAX_ATROUS_LEVELS: u32 = 5;

/// HW-RT rung 3a: the byte size of the à-trous edge-stop UBO — the RHI-layer MIRROR of
/// `boyko_render::RESOLVED_SHADOW_DENOISE_BYTES` (`size_of::<ResolvedShadowDenoise>()`, one std140
/// vec4 = 16 B). The RHI mints the per-FIF `shadow_denoise_ubo` ring at this size (`sigma_z` @0,
/// `sigma_n` @4, pad @8/@12); the host writes `ResolvedShadowDenoise`'s 16 bytes into the fenced
/// slot. Kept equal to `boyko_render::RESOLVED_SHADOW_DENOISE_BYTES` (16).
#[cfg(feature = "hwrt")]
pub const SHADOW_DENOISE_UBO_BYTES: u64 = 16;

/// HW-RT Rung 3b: the byte size of the temporal reproject UBO — the RHI-layer MIRROR of
/// `boyko_render::RESOLVED_TEMPORAL_SHADOW_BYTES` (`size_of::<ResolvedTemporalShadow>()`, one std140
/// vec4 = 16 B). A SEPARATE carrier from `SHADOW_DENOISE_UBO_BYTES` so the shipped à-trous UBO byte
/// stream stays untouched. The RHI mints the per-FIF `temporal_shadow_ubo` ring at this size
/// (`feedback_max` @0, `feedback_min` @4, `variance_gamma` @8, `depth_tol` @12); the host writes
/// `ResolvedTemporalShadow`'s 16 bytes into the fenced slot. Kept equal to
/// `boyko_render::RESOLVED_TEMPORAL_SHADOW_BYTES` (16).
#[cfg(feature = "hwrt")]
pub const TEMPORAL_SHADOW_UBO_BYTES: u64 = 16;

/// Errors from surface / swapchain / present operations.
#[derive(Debug)]
pub enum SwapchainError {
    /// The context was not built windowed ([`crate::device::InstanceConfig::windowed`]
    /// was `false`), so the surface/swapchain command tables are absent.
    NotWindowed,
    /// No queue family supports presentation to this surface.
    NoPresentQueue,
    /// The surface advertised no usable color format.
    NoSuitableFormat,
    /// The surface reported a zero extent (e.g. a minimized window) — defer
    /// rendering until it is non-zero again.
    ZeroExtent,
    /// A Vulkan command returned a non-success `VkResult`.
    VkError(&'static str, VkResult),
    /// The rung-7 scene's per-extent depth image could not be (re)created (resource
    /// creation through the RHI texture path failed).
    DepthImage(crate::error::VulkanError),
}

/// The single-color, single-mip, single-layer subresource range used for every
/// swapchain image view + barrier.
pub(crate) const COLOR_SUBRESOURCE_RANGE: VkImageSubresourceRange = VkImageSubresourceRange {
    aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// The single-mip, single-layer DEPTH-aspect subresource range used for the
/// rung-7 scene depth image's barrier (the depth counterpart of
/// [`COLOR_SUBRESOURCE_RANGE`]).
pub(crate) const DEPTH_SUBRESOURCE_RANGE: VkImageSubresourceRange = VkImageSubresourceRange {
    aspect_mask: VK_IMAGE_ASPECT_DEPTH_BIT,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};
