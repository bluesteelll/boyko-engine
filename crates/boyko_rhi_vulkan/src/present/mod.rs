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
    AaActivation, BrickActivation, CsmDepthActivation, DdgiUpdateActivation,
    GBUFFER_IDENTITY_INSTANCE, GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES, GBufferMeshDraw,
    GBufferScene, InterpActivation, PunctualDepthActivation, SCENE_MVP_BYTES, SampledComposite,
    Scene, SmaaActivation, SsaaActivation, SsaoActivation, TaaActivation, UiPass,
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

/// The SSAO edge-avoiding à-trous denoise chain: the max pass count the recorder can dispatch —
/// the RHI-layer MIRROR of `boyko_render::ssao_config::MAX_SSAO_ATROUS_LEVELS` (the RHI cannot
/// depend on `boyko_render`, mirroring [`MAX_ATROUS_LEVELS`]'s duplication rationale). Kept equal
/// (5); a cross-crate integration test asserts the equality. Software (NOT `hwrt`-gated) — unlike
/// [`MAX_ATROUS_LEVELS`], every leg builds this. [`ssao_atrous_step`]'s 5 ROLE-KEYED
/// pipelines/sets are N-INDEPENDENT, so a level count up to this max is a LIVE per-frame choice
/// (no rebuild) — see `present::scene_types::SsaoActivation`.
pub const MAX_SSAO_ATROUS_LEVELS: u32 = 5;

/// The SSAO à-trous chain's C1 role selection for dispatch level `level` of `n` total passes
/// (`n` in `{0} ∪ {2..=`[`MAX_SSAO_ATROUS_LEVELS`]`}` — `boyko_render::SsaoConfig::clamped_atrous_levels`'s
/// contract). PURE (no GPU handle): the recorder ([`crate::present::passes::gbuffer`]), the
/// descriptor-set builder ([`GBufferTargets::build_ssao_atrous_sets`]), the framegraph declarator
/// ([`GbufferPassPlan::ssao_atrous`]'s ResId chain), and any headless test harness that dispatches
/// the SAME N-pass chain all call THIS one function for the level→role mapping, so they can never
/// diverge.
///
/// Because the intermediate ping-pong is TWO rings (not one uniform format like the shadow
/// à-trous), the two chain ENDPOINTS need DIFFERENT pipeline variants from the interior:
/// - `level == 0`: [`AtrousStepRole::Read8`] — reads the frozen R8 `gSsao` endpoint, writes ring 0.
/// - `level == n - 1` (`n >= 2`): [`AtrousStepRole::Write8`] — reads `ring[in_ring]`, writes BACK
///   into the frozen R8 `gSsao` endpoint.
/// - otherwise (`0 < level < n - 1`): [`AtrousStepRole::Interior`] — reads `ring[in_ring]`, writes
///   `ring[1 - in_ring]` (both R16).
///
/// `in_ring = (level - 1) % 2` for every non-`Read8` role: level `k`'s input is whatever level
/// `k - 1` wrote (level 0 always writes ring 0, so level 1 reads ring 0 == `(1-1)%2`, level 2
/// reads ring 1 == `(2-1)%2`, etc. — a uniform ping-pong once you fold the two R8 endpoints in as
/// virtual "ring -1" / "ring n" slots).
///
/// # Panics (debug only)
///
/// `debug_assert!`s `level < n` — the caller's loop bound (`0..n`) already guarantees this; a
/// violation is a caller bug, not a runtime condition.
#[inline]
pub fn ssao_atrous_step(level: u32, n: u32) -> AtrousStepRole {
    debug_assert!(
        level < n,
        "invariant: ssao_atrous_step is called for level in 0..n"
    );
    if level == 0 {
        AtrousStepRole::Read8
    } else if level == n - 1 {
        AtrousStepRole::Write8 { in_ring: (level - 1) % 2 }
    } else {
        AtrousStepRole::Interior { in_ring: (level - 1) % 2 }
    }
}

/// The role [`ssao_atrous_step`] selects for one SSAO à-trous dispatch level — which of the 5
/// role-keyed pipeline/descriptor-set pairs ([`present::scene_types::SsaoActivation`]'s
/// `atrous_read8_pipeline`/`atrous_interior_pipeline`/`atrous_write8_pipeline` +
/// [`GBufferTargets`]'s five `ssao_atrous_*_set` rings) the caller binds for that level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtrousStepRole {
    /// Level 0: `gAoIn` = the frozen R8 `gSsao` endpoint, `gAoOut` = R16 ring 0. Selects the
    /// `read8` pipeline variant + the `ssao_atrous_read8_set`.
    Read8,
    /// An interior level (`0 < level < n - 1`): `gAoIn` = R16 `ring[in_ring]`, `gAoOut` = R16
    /// `ring[1 - in_ring]`. Selects the `interior` pipeline variant + `ssao_atrous_interior_from0_set`
    /// (`in_ring == 0`) or `ssao_atrous_interior_from1_set` (`in_ring == 1`).
    Interior {
        /// The ring index (0 or 1) `gAoIn` reads from; `gAoOut` writes the OTHER ring.
        in_ring: u32,
    },
    /// The last level (`level == n - 1`, `n >= 2`): `gAoIn` = R16 `ring[in_ring]`, `gAoOut` = the
    /// frozen R8 `gSsao` endpoint (the write-back). Selects the `write8` pipeline variant +
    /// `ssao_atrous_write8_from0_set` (`in_ring == 0`) or `ssao_atrous_write8_from1_set`
    /// (`in_ring == 1`).
    Write8 {
        /// The ring index (0 or 1) `gAoIn` reads from.
        in_ring: u32,
    },
}

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

/// Anti-aliasing Stage 4 (TAA W5): the byte size of the TAA resolve's tunables UBO — the
/// RHI-layer MIRROR of `boyko_render::RESOLVED_TAA_BYTES` (`size_of::<ResolvedTaa>()`, one std140
/// vec4 = 16 B). The RHI cannot depend on `boyko_render` (the render crate sits ABOVE it), so the
/// value is duplicated here — mirrors [`TEMPORAL_SHADOW_UBO_BYTES`]'s pattern, UNCONDITIONAL
/// (TAA is NOT `hwrt`-gated). The RHI mints the per-FIF `taa_ubo` ring at this size
/// (`default_blend` @0, `min_blend` @4, `variance_gamma` @8, pad @12); the host writes
/// `ResolvedTaa`'s 16 bytes into the fenced slot.
pub const TAA_UBO_BYTES: u64 = 16;

/// Anti-aliasing Stage 4 (TAA W5): the byte size of the TAA resolve's DEDICATED `MotionCam` UBO —
/// the RHI-layer MIRROR of `boyko_render::MOTION_CAM_UBO_BYTES` (two `float4x4`, 128 B). A
/// SEPARATE ring from the hwrt mesh-shadow `motion_cam_ubo` (see `TaaActivation`'s doc for the
/// "why a dedicated ring" rationale) — UNCONDITIONAL (both feature legs).
pub const TAA_MOTION_CAM_UBO_BYTES: u64 = 128;

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
