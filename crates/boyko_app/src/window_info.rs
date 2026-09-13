//! [`WindowInfo`] — the world-resident window-size snapshot (host plan D7) —
//! and the runner's two host probes beside it: [`HostFrameStats`], per frame (R4),
//! and [`HostTeardownStats`], once per run after teardown.

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
    /// Frames on which the punctual (spot/point) depth pass was armed
    /// (`GBufferScene::atlas_punctual == Some` — a fitted shadow atlas
    /// (`ResolvedShadowAtlas::mode_word == 1`, at least one `CastsPunctualShadow`
    /// light slotted) AND live caster batches). Zero on a scene with no
    /// `CastsPunctualShadow` light (punctual OFF — byte-identical to the pre-rung path).
    pub punctual_armed_frames: u64,
    /// Frames on which the interpolation pre-pass was armed (host plan R5): the
    /// pair gather produced instances (`MeshRenderScratch::pair_ring` non-empty),
    /// so the runner uploaded the pair ring, armed `GBufferScene::interp`, and the
    /// raster VS read the interpolated draw SSBO. Zero on a scene with no
    /// `GpuTransform3D` body (interp OFF — byte-identical to pre-R5).
    pub interp_armed_frames: u64,
}

/// How many host-pool sub-allocations one run can ask [`HostTeardownStats`] to watch.
pub const HOST_TEARDOWN_WATCH_SLOTS: usize = 4;

/// One watched host-pool sub-allocation in [`HostTeardownStats::watch`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostAllocationWatch {
    /// The allocation to watch, as a host-visible `BoundBuffer`'s `(block, offset)` — the key
    /// `VulkanContext::host_allocation_is_live` reads. Set by a caller during the run; `None`
    /// leaves the entry unsampled.
    pub key: Option<(u32, u64)>,
    /// Whether that key still named a live host-pool sub-allocation at the runner's
    /// post-teardown sample. Written only by that sample, and only for a `Some` key. A `true`
    /// is either the watched buffer or a later allocation that re-used its offset, so it can
    /// only err toward reporting a leak.
    pub live_at_teardown: bool,
}

/// What the windowed runner's teardown left alive — sampled once per run, after every host
/// destroy and before the context's own `Drop` frees the memory pools and destroys the device.
///
/// The post-run twin of [`HostFrameStats`]: it lets device tests assert that teardown destroyed
/// what boot created. The runner inserts it with its default at the sample if it is absent; a
/// caller that watches allocations inserts it first and sets [`watch`](Self::watch) keys during
/// the run. Not consumed by any engine system.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostTeardownStats {
    /// `true` only once the runner's sample has run — a record that reads `false` measured
    /// nothing.
    pub sampled: bool,
    /// Whether `GpuDevice` was still in the World at the sample. Teardown evicts it as its last
    /// step, so `false` places the sample after that eviction.
    pub gpu_device_present_at_sample: bool,
    /// Whether `MeshGeometryTableSlot` was in the World and held a table at the sample.
    pub geometry_table_present_at_sample: bool,
    /// Live host-visible sub-allocations at the sample, summed over every block (saturated to
    /// `u32::MAX`).
    pub host_allocations_live: u32,
    /// Live device-local sub-allocations at the sample, summed the same way.
    pub device_allocations_live: u32,
    /// `VulkanContext::persistent_descriptor_pools` at the sample.
    pub persistent_descriptor_pools_live: u32,
    /// Host-pool sub-allocations checked at the sample; entries with a `None` key are skipped.
    pub watch: [HostAllocationWatch; HOST_TEARDOWN_WATCH_SLOTS],
}
