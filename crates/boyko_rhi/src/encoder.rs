//! The [`RhiCommandEncoder`] operational trait: the hot command-recording path.
//!
//! No `dyn`, no per-command allocation. Every method monomorphizes to a direct
//! `(fns.cmd_*)` indirect call — byte-identical codegen to the current inherent
//! Vulkan methods (plan O4). `pipeline_barrier` is **buffer-only** in Phase 1
//! (plan D3): explicit caller-side barriers per plan §5.5, no auto-tracking.

use crate::api::RhiApi;
use crate::descriptor::{
    BarrierDesc, BufferCopy, BufferImageCopy, ImageBarrierDesc, RenderArea, RenderingDesc, Viewport,
};
use crate::enums::{ImageLayout, IndexType, ShaderStage};
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

    /// Binds a graphics pipeline for subsequent [`Self::draw`] calls (Phase-6 S0
    /// rung 2). Must be called inside a [`Self::begin_rendering`] scope whose color
    /// attachment formats match the pipeline's declared `color_format`.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (mirroring [`Self::bind_compute_pipeline`]).
    #[cold]
    #[inline(never)]
    fn bind_graphics_pipeline(&mut self, _pipeline: &A::GraphicsPipeline) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Sets the dynamic viewport (Phase-6 S0 rung 2). The pipeline is created with
    /// dynamic viewport state, so this must be recorded before [`Self::draw`].
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it.
    #[cold]
    #[inline(never)]
    fn set_viewport(&mut self, _viewport: &Viewport) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Sets the dynamic scissor rectangle (Phase-6 S0 rung 2). The pipeline is
    /// created with dynamic scissor state, so this must be recorded before
    /// [`Self::draw`].
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it.
    #[cold]
    #[inline(never)]
    fn set_scissor(&mut self, _scissor: &RenderArea) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Binds `buffer` as the vertex buffer at `binding`, reading from byte `offset`
    /// (Phase-6 S0 rung 3). Must be recorded before a [`Self::draw`] that consumes a
    /// vertex layout. Rung 3 binds binding `0` at offset `0`.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdBindVertexBuffers`).
    #[cold]
    #[inline(never)]
    fn bind_vertex_buffer(&mut self, _buffer: &A::Buffer, _binding: u32, _offset: u64) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Binds `buffer` as the index buffer (from byte `offset`, with index width
    /// `index_type`) for a subsequent indexed draw (Phase-6 S0 rung-3 seam). Rung 3
    /// itself draws non-indexed, so the foundation does not call this yet; it is
    /// defined so the encoder surface + ABI are stable for the indexed-draw rung.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdBindIndexBuffer`).
    #[cold]
    #[inline(never)]
    fn bind_index_buffer(&mut self, _buffer: &A::Buffer, _offset: u64, _index_type: IndexType) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Records a push-constant update against a **graphics** pipeline's layout
    /// (Phase-6 S0 rung 3 — the MVP `float4x4`). Distinct from [`Self::push_constants`]
    /// (which targets the fixed compute layout): this reads the layout from the
    /// passed graphics `pipeline`, so the MVP can be pushed to the rung-3 pipeline's
    /// `VERTEX`-stage range without touching the compute path.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdPushConstants` against the pipeline's layout).
    #[cold]
    #[inline(never)]
    fn push_graphics_constants(
        &mut self,
        _pipeline: &A::GraphicsPipeline,
        _stage: ShaderStage,
        _offset: u32,
        _bytes: &[u8],
    ) {
        // Phase-6 S0 default seam: overridden by the Vulkan backend.
    }

    /// Records a non-indexed draw of `vertex_count` vertices in `instance_count`
    /// instances, starting at `first_vertex` / `first_instance` (Phase-6 S0 rung
    /// 2). Rung 2 issues `draw(3, 1, 0, 0)` — one triangle, vertex positions
    /// generated by the vertex shader from the vertex index (no vertex buffer).
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdDraw`).
    #[cold]
    #[inline(never)]
    fn draw(
        &mut self,
        _vertex_count: u32,
        _instance_count: u32,
        _first_vertex: u32,
        _first_instance: u32,
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
