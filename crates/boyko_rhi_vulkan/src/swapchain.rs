//! Compatibility re-export shim for the former monolithic `swapchain.rs`.
//!
//! The Vulkan surface / swapchain / present implementation was decomposed into the
//! [`crate::present`] module tree (audit W4). This module preserves every existing
//! `crate::swapchain::X` / `boyko_rhi_vulkan::swapchain::X` path by re-exporting the
//! public surface of [`crate::present`] unchanged.

pub use crate::present::{
    AaActivation, BrickActivation, ClusterCullHierDispatch, CsmDepthActivation, DdgiUpdateActivation,
    FRAMES_IN_FLIGHT, FrameWriteToken, GBUFFER_IDENTITY_INSTANCE, GBUFFER_INSTANCE_MODEL_BYTES,
    GBUFFER_PUSH_BYTES, GBufferFrame, GBufferMeshDraw, GBufferScene, GBufferTargets,
    InterpActivation, PASS_COUNT, PunctualDepthActivation, RcasActivation, Renderer,
    ResolvedRenderPathGpu, SCENE_MVP_BYTES, SV0_PASS_COUNT, SampledComposite, Scene,
    SmaaActivation, SsaaActivation, SsaoActivation, Surface, Sv0TimedPass, Sv0TimestampCollector,
    Swapchain, SwapchainError, TaaActivation, TimedPass, TimestampCollector, UiPass, VB_PASS_COUNT,
    VbTimedPass, VbTimestampCollector, ViewtFromDepthActivation, ViewtFromVbDepthActivation,
};
#[cfg(feature = "hwrt")]
pub use crate::present::{ShadowVisActivation, TlasBuildActivation};
