//! [`WindowInfo`] — the world-resident window-size snapshot (host plan D7).

use boyko_macros::Resource;

/// The window's client size as observed by the host, in physical pixels.
///
/// # One-frame-stale contract (host plan D7)
///
/// The runner writes this POST-present (runner-frame step 8), so a `Main`
/// system reads the PREVIOUS frame's size — never mid-frame-torn, one frame
/// stale by design. Inert in v1: the composite extent is boot-fixed (a window
/// resize only recreates the swapchain and the present blit clamps), so no v1
/// engine system consumes this; it exists for user HUD/layout logic and for
/// the v2 dynamic-resize tracking to grow into.
///
/// Before the first present it holds the boot client size.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowInfo {
    /// Client-area width, physical pixels (previous frame's observation).
    pub width: u32,
    /// Client-area height, physical pixels (previous frame's observation).
    pub height: u32,
}
