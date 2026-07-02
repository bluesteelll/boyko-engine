//! [`WindowInfo`] — the world-resident window-size snapshot (host plan D7) —
//! and [`HostFrameStats`], its WindowInfo-adjacent per-frame host probe (R4).

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

/// The host's per-frame observability counters (host plan R4) — a
/// [`WindowInfo`]-adjacent world resource written by the runner POST-present
/// (the same one-frame-stale contract), so headless smokes can assert host
/// decisions that otherwise live only on the runner's stack: whether the light
/// generation protocol actually gated uploads and whether the CSM depth pass
/// was armed. Monotonic counters, zero per-frame allocation (three integer
/// stores per presented frame). Not consumed by any engine system.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostFrameStats {
    /// Frames that reached the post-render publish step (presented or
    /// recreate-skipped — the loop's step 8).
    pub frames: u64,
    /// Frames whose fenced slot's light staging was rewritten
    /// (`light_uploaded_gen[s]` lagged `LightTableGeneration`). Under the D5
    /// protocol this is `<= 2 + 2 × writer-side bumps`, and strictly less than
    /// `frames` on any run longer than the catch-up window — the gating proof.
    pub light_uploads: u64,
    /// Frames on which the cascade depth pass was armed
    /// (`GBufferScene::csm == Some` — a fitted sun AND live caster batches).
    pub csm_armed_frames: u64,
}
