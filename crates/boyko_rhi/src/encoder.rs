//! The [`RhiCommandEncoder`] operational trait: the hot command-recording path.
//!
//! No `dyn`, no per-command allocation. Every method monomorphizes to a direct
//! `(fns.cmd_*)` indirect call — byte-identical codegen to the current inherent
//! Vulkan methods (plan O4). `pipeline_barrier` is **buffer-only** in Phase 1
//! (plan D3): explicit caller-side barriers per plan §5.5, no auto-tracking.

use crate::api::RhiApi;
use crate::descriptor::{
    BarrierDesc, BufferCopy, BufferImageCopy, ImageBarrierDesc, RenderingDesc,
};
use crate::enums::{ImageLayout, ShaderStage};
use crate::error::RhiError;

/// Records commands into a one-time-submit command buffer.
///
/// The encoder owns its command pool + command buffer + descriptor pool + the
/// fixed compute descriptor set (built once at `create_command_encoder`, per
/// plan Q1 — no per-record descriptor-set rebuild).
pub trait RhiCommandEncoder<A: RhiApi> {
    /// One unified per-backend error type (plan D4); bound is `From<RhiError>`
    /// only — see [`crate::device::RhiDevice::Error`].
    type Error: core::fmt::Debug + From<RhiError>;

    /// Resets and begins recording (one-time-submit).
    fn begin(&mut self) -> Result<(), Self::Error>;

    /// Ends recording.
    fn end(&mut self) -> Result<(), Self::Error>;

    /// Binds a compute pipeline for subsequent dispatches.
    fn bind_compute_pipeline(&mut self, pipeline: &A::ComputePipeline);

    /// Binds `buffer` as the storage buffer at `(set, binding)`.
    ///
    /// The encoder updates its cached descriptor set only when the bound buffer
    /// differs from the last binding (plan Q1).
    fn bind_storage_buffer(&mut self, buffer: &A::Buffer, set: u32, binding: u32);

    /// Records a push-constant update for `stage` at byte `offset`.
    fn push_constants(&mut self, stage: ShaderStage, offset: u32, bytes: &[u8]);

    /// Records a compute dispatch of `gx * gy * gz` workgroups.
    fn dispatch(&mut self, gx: u32, gy: u32, gz: u32);

    /// Records an explicit buffer pipeline barrier (plan §5.5; no auto-tracking).
    fn pipeline_barrier(&mut self, barrier: &BarrierDesc<A>);

    // ===== DEFAULT-BODY SEAM (Phase 5 staging copy; Mock + ABI untouched) =====

    /// Records a buffer-to-buffer copy of `regions` from `src` to `dst`.
    ///
    /// The Phase-5 `GpuColumn` staging upload (host-visible → device-local) and
    /// the test-only readback (device-local → host-visible) go through this.
    /// Only a backend that supports device-local transfers (Vulkan) overrides it;
    /// the default body is a no-op marked `#[cold] #[inline(never)]` so it never
    /// touches the hot recording path's I-cache when not overridden, mirroring
    /// [`Self::dispatch_indirect`]. The Mock backend and the trait ABI are
    /// therefore unaffected by adding it.
    #[cold]
    #[inline(never)]
    fn copy_buffer(&mut self, _src: &A::Buffer, _dst: &A::Buffer, _regions: &[BufferCopy]) {
        // Phase-5 default seam: a backend without a device-local transfer path
        // (e.g. the Mock) leaves this a no-op; the Vulkan backend overrides it.
    }

    // ===== GRAPHICS-SURFACE SEAM (Phase 6 S0; default bodies keep Mock + ABI) =====

    /// Records an image-layout transition (the Phase-2-3 `ImageBarrier` seam, RHI
    /// plan D3/C1, needed by the Phase-6 S0 dynamic-rendering path).
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]` so a backend
    /// without an image path (the Mock) leaves it a no-op and the trait ABI is
    /// unaffected; the Vulkan backend overrides it (mirroring [`Self::copy_buffer`]).
    #[cold]
    #[inline(never)]
    fn image_barrier(&mut self, _barrier: &ImageBarrierDesc<A>) {
        // Phase-6 S0 default seam: a backend without an image path leaves this a
        // no-op; the Vulkan backend overrides it.
    }

    /// Begins a Vulkan 1.3 dynamic-rendering scope (no `VkRenderPass`), binding the
    /// color attachments in `desc` with their load/store ops + clear values.
    /// Must be paired with [`Self::end_rendering`]. Seam: Phase 6 S0.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it.
    #[cold]
    #[inline(never)]
    fn begin_rendering(&mut self, _desc: &RenderingDesc<A>) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Ends the dynamic-rendering scope opened by [`Self::begin_rendering`].
    /// Seam: Phase 6 S0.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it.
    #[cold]
    #[inline(never)]
    fn end_rendering(&mut self) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Records an image→buffer copy of `regions` from `src` (currently in
    /// `src_layout`, typically [`ImageLayout::TransferSrcOptimal`]) to `dst`.
    /// The S0 offscreen golden-readback transfer (the image counterpart of
    /// [`Self::copy_buffer`]). Seam: Phase 6 S0.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it.
    #[cold]
    #[inline(never)]
    fn copy_image_to_buffer(
        &mut self,
        _src: &A::Texture,
        _src_layout: ImageLayout,
        _dst: &A::Buffer,
        _regions: &[BufferImageCopy],
    ) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    // ===== DEFERRED SEAM (Phase 6+) =====

    /// Records an indirect compute dispatch reading group counts from `buffer` at
    /// byte `offset`. Seam: Phase 6+.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]` so it never
    /// touches the hot recording path's I-cache when not overridden; a backend
    /// fills it in when indirect dispatch lands.
    #[cold]
    #[inline(never)]
    fn dispatch_indirect(&mut self, _buffer: &A::Buffer, _offset: u64) {
        // Phase-6+ seam: no foundation code calls this.
    }
}
