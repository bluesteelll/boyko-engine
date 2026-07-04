//! Compatibility re-export shim for the former monolithic `swapchain.rs`.
//!
//! The Vulkan surface / swapchain / present implementation was decomposed into the
//! [`crate::present`] module tree (audit W4). This module preserves every existing
//! `crate::swapchain::X` / `boyko_rhi_vulkan::swapchain::X` path by re-exporting the
//! public surface of [`crate::present`] unchanged.

pub use crate::present::{
    BrickActivation, CsmDepthActivation, DdgiUpdateActivation, FRAMES_IN_FLIGHT, FrameWriteToken,
    GBUFFER_IDENTITY_INSTANCE, GBUFFER_INSTANCE_MODEL_BYTES, GBUFFER_PUSH_BYTES, GBufferFrame,
    GBufferMeshDraw, GBufferScene, GBufferTargets, InterpActivation, PASS_COUNT,
    PunctualDepthActivation, Renderer, SCENE_MVP_BYTES, SampledComposite, Scene, SsaoActivation,
    Surface, Swapchain, SwapchainError, TimedPass, TimestampCollector, UiPass,
};
#[cfg(feature = "hwrt")]
pub use crate::present::TlasBuildActivation;
