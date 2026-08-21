//! The [`RhiCommandEncoder`] operational trait: the hot command-recording path.
//!
//! No `dyn`, no per-command allocation. Every method monomorphizes to a direct
//! `(fns.cmd_*)` indirect call — byte-identical codegen to the current inherent
//! Vulkan methods (plan O4). `pipeline_barrier` is **buffer-only** in Phase 1
//! (plan D3): explicit caller-side barriers per plan §5.5, no auto-tracking.

use crate::api::RhiApi;
use crate::descriptor::{
    AsBuildEntry, BarrierDesc, BufferCopy, BufferImageCopy, ImageBarrierDesc, ImageBlitDesc,
    RenderArea, RenderingDesc, Viewport,
};
use crate::enums::{ImageLayout, IndexType, ShaderStage, TimestampStage};
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

    /// Records a buffer→image copy of `regions` from `src` to `dst` (which must be
    /// in `dst_layout`, typically [`ImageLayout::TransferDstOptimal`]). The rung-11
    /// upload of the compute composite's packed-RGBA pixel region into a SAMPLED
    /// texture, so a fullscreen-sample pass can present it (the symmetric
    /// counterpart of [`Self::copy_image_to_buffer`]). Seam: Phase 6 S1 rung 11.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdCopyBufferToImage`, reusing the same
    /// `BufferImageCopy` region type as [`Self::copy_image_to_buffer`]).
    #[cold]
    #[inline(never)]
    fn copy_buffer_to_image(
        &mut self,
        _src: &A::Buffer,
        _dst: &A::Texture,
        _dst_layout: ImageLayout,
        _regions: &[BufferImageCopy],
    ) {
        // Phase-6 S1 default seam: overridden by the Vulkan backend.
    }

    /// Records a LINEAR-filtered blit from one mip level of an image to another mip
    /// level of the SAME image (`vkCmdBlitImage`, textured-PBR T2 Decision D3) — the
    /// mip-chain generation step of a staged texture upload: level `i` is downsampled
    /// from level `i - 1` by GPU-filtered blit rather than a re-upload. Both regions
    /// are the full extent of their mip level; `desc.src_layout`/`desc.dst_layout` are
    /// the layouts the caller transitioned each level to via a prior
    /// [`Self::image_barrier`] (typically `TransferSrcOptimal` / `TransferDstOptimal`).
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (mirrors [`Self::copy_buffer_to_image`]).
    #[cold]
    #[inline(never)]
    fn blit_image(&mut self, _desc: &ImageBlitDesc<A>) {
        // Textured-PBR T2 default seam: overridden by the Vulkan backend.
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

    /// Binds `group` (a descriptor set) at `set 0` of `pipeline`'s layout for the
    /// GRAPHICS bind point (Phase-6 S0 rung 5). Must be recorded after
    /// [`Self::bind_graphics_pipeline`] and before a [`Self::draw`] that samples the
    /// bound texture; `pipeline` supplies the pipeline layout the set is bound
    /// against (the layout must have been built with the same bind-group layout via
    /// `GraphicsPipelineDesc::bind_group_layout`).
    ///
    /// Since `docs/UI-PLAN-SPRITES.md` S3 (decision S-D3) this is a THIN, provided
    /// wrapper over [`Self::bind_descriptor_set_at`]`(0, …)` — the backend overrides
    /// the general verb, and this signature is unchanged, so no existing call site
    /// moved.
    #[inline]
    fn bind_descriptor_set(&mut self, group: &A::BindGroup, pipeline: &A::GraphicsPipeline) {
        self.bind_descriptor_set_at(0, group, pipeline);
    }

    /// Binds `group` at descriptor-set index `set_index` of `pipeline`'s layout for
    /// the GRAPHICS bind point (`docs/UI-PLAN-SPRITES.md` rung S3, decision S-D3).
    ///
    /// This is the general verb; [`Self::bind_descriptor_set`] is its
    /// `set_index == 0` case, and the ONLY one a backend overrides.
    /// `pipeline` must have been created with a MULTI-SET layout that declares
    /// `set_index` (`VulkanContext::create_graphics_pipeline_bindless` and its
    /// siblings), and `group`'s own layout must be compatible with the layout
    /// declared at that index — a shader that statically uses set *n* with no set
    /// bound there is `VUID-vkCmdDraw-None-08600`, not a silent no-op.
    ///
    /// # Why this exists (and why it is not a Vulkan-only escape hatch)
    ///
    /// Binding a second set was expressible ONLY on the concrete on-screen path
    /// (`present_blit.rs`'s raw `cmd_bind_descriptor_sets`), while the offscreen
    /// goldens — the ones that run without a display — drive the SAME recorder
    /// through this trait. Without this verb the two recorders would be
    /// structurally different at the exact place they must agree, and the sprite
    /// path would be untestable on a device-less machine (S-D3's rejected options
    /// (a) and (b); risk **M3-c**).
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdBindDescriptorSets` with `firstSet =
    /// set_index`, GRAPHICS bind point).
    #[cold]
    #[inline(never)]
    fn bind_descriptor_set_at(
        &mut self,
        _set_index: u32,
        _group: &A::BindGroup,
        _pipeline: &A::GraphicsPipeline,
    ) {
        // Default seam: overridden by the Vulkan backend.
    }

    /// Binds `group` (a descriptor set) at `set 0` of `compute_pipeline`'s layout for
    /// the COMPUTE bind point (Render P1a). Must be recorded after
    /// [`Self::bind_compute_pipeline`] and before a [`Self::dispatch`] that reads the
    /// bound vocabulary set; `compute_pipeline` supplies the pipeline layout the set
    /// is bound against (the layout must have been built with the same bind-group
    /// layout via `ComputePipelineDesc::bind_group_layout`). The COMPUTE counterpart
    /// of [`Self::bind_descriptor_set`]; it does NOT touch the encoder's fixed
    /// single-STORAGE_BUFFER set used by [`Self::bind_storage_buffer`] (the two
    /// coexist — the packed-buffer offscreen path keeps the fixed set).
    ///
    /// # Push constants on a vocabulary pipeline (review O1)
    ///
    /// [`Self::push_constants`] records against the encoder's cached SHARED compute
    /// pipeline layout (the fixed/`None`-layout path). A vocabulary-compute pipeline
    /// (created with [`crate::descriptor::ComputePipelineDesc::bind_group_layout`]
    /// `== Some`) is bound here against its DEDICATED layout, which is NOT
    /// push-constant-compatible with the shared layout under Vulkan — calling
    /// [`Self::push_constants`] while a vocabulary pipeline is bound is a validation
    /// error. A vocabulary pipeline must push against ITS OWN layout (a dedicated
    /// `push_constants(&ComputePipeline, …)` variant is added in P1b for the
    /// marcher's camera block); the current shared-layout [`Self::push_constants`]
    /// stays valid only for the fixed/`None`-layout path. P1a wires no push on the
    /// vocabulary path.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdBindDescriptorSets`, COMPUTE bind point).
    #[cold]
    #[inline(never)]
    fn bind_descriptor_set_compute(
        &mut self,
        _group: &A::BindGroup,
        _compute_pipeline: &A::ComputePipeline,
    ) {
        // Render P1a default seam: overridden by the Vulkan backend.
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

    /// Records a push-constant update against a **compute** pipeline's OWN layout
    /// (Render P4b — the marcher's `coarse_enabled` gate). The COMPUTE counterpart of
    /// [`Self::push_graphics_constants`]: it reads the layout from the passed compute
    /// `pipeline`, so a VOCABULARY-compute pipeline (created with
    /// [`crate::descriptor::ComputePipelineDesc::bind_group_layout`] `== Some`) can push
    /// against its DEDICATED layout — the one its bind group is bound against — instead
    /// of the device-shared layout [`Self::push_constants`] targets (which is NOT
    /// push-/set-compatible with a vocabulary pipeline → a validation error). The fixed
    /// (`None`-layout) packed-buffer path keeps using [`Self::push_constants`].
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdPushConstants` against the pipeline's own layout).
    #[cold]
    #[inline(never)]
    fn push_compute_constants(
        &mut self,
        _pipeline: &A::ComputePipeline,
        _stage: ShaderStage,
        _offset: u32,
        _bytes: &[u8],
    ) {
        // Render P4b default seam: overridden by the Vulkan backend.
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

    /// Records an indexed draw of `index_count` indices in `instance_count`
    /// instances, starting at `first_index` in the bound index buffer; each fetched
    /// index has `vertex_offset` (signed) added before the vertex-buffer lookup, and
    /// instancing starts at `first_instance` (mesh M0).
    ///
    /// Requires a bound index buffer ([`Self::bind_index_buffer`]) and the vertex
    /// buffer(s) the indices reference ([`Self::bind_vertex_buffer`]); records
    /// `vkCmdDrawIndexed`. This is the indexed counterpart of [`Self::draw`].
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan
    /// backend overrides it (`vkCmdDrawIndexed`).
    #[cold]
    #[inline(never)]
    fn draw_indexed(
        &mut self,
        _index_count: u32,
        _instance_count: u32,
        _first_index: u32,
        _vertex_offset: i32,
        _first_instance: u32,
    ) {
        // Mesh M0 default seam: overridden by the Vulkan backend.
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

    // ===== GPU TIMESTAMP-QUERY SEAM (HW-RT rung R0; default bodies keep Mock + ABI) =====

    /// Records a reset of `count` queries starting at `first` in `pool`
    /// (`vkCmdResetQueryPool`, HW-RT rung R0). A TIMESTAMP query is UNDEFINED at pool
    /// creation and stays undefined until reset, so this MUST run before the frame's first
    /// [`Self::write_timestamp`] targeting those queries — and MUST be recorded **OUTSIDE**
    /// any render / dynamic-rendering scope (`VUID-vkCmdResetQueryPool-renderpass`); a
    /// compute-only prologue is trivially legal.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]` so a backend without a
    /// query path (the Mock) leaves it a no-op and the trait ABI is unaffected; the Vulkan
    /// backend overrides it (mirroring [`Self::image_barrier`]).
    #[cold]
    #[inline(never)]
    fn reset_query_pool(&mut self, _pool: &A::QueryPool, _first: u32, _count: u32) {
        // HW-RT R0 default seam: a backend without a query path leaves this a no-op; the
        // Vulkan backend overrides it.
    }

    /// Records a timestamp write into query `index` of `pool` at pipeline stage `stage`
    /// (`vkCmdWriteTimestamp`, HW-RT rung R0). `index` MUST have been reset this frame via
    /// [`Self::reset_query_pool`]. The profiler-standard bracket writes
    /// [`TimestampStage::TopOfPipe`] at a pass's open and [`TimestampStage::BottomOfPipe`]
    /// at its close.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan backend
    /// overrides it.
    #[cold]
    #[inline(never)]
    fn write_timestamp(&mut self, _pool: &A::QueryPool, _stage: TimestampStage, _index: u32) {
        // HW-RT R0 default seam: a backend without a query path leaves this a no-op; the
        // Vulkan backend overrides it.
    }

    // ===== HW-RT ACCELERATION-STRUCTURE SEAM (rung R2a-1; default bodies keep Mock + ABI) =====
    // Declared UNGATED so the trait ABI is stable across phases (mirroring the timestamp
    // seam). Default bodies are no-ops marked `#[cold] #[inline(never)]`; the Vulkan
    // backend overrides them ONLY under `feature="hwrt"`. With `hwrt` OFF no consumer
    // records these (the resolve stays software), so they never execute — byte-identical.

    /// Records `entries.len()` acceleration-structure builds into the command buffer
    /// (`vkCmdBuildAccelerationStructuresKHR`, HW-RT rung R2a-1). Each [`AsBuildEntry`]
    /// pairs a target level + geometry (device addresses) + a scratch device address; the
    /// backend fills the `VkAccelerationStructureBuildGeometryInfoKHR` +
    /// `BuildRangeInfoKHR` per entry, writing the built structure at the entry's
    /// destination AS. `dest_addresses[i]` is the device address of the AS that entry `i`
    /// builds into (parallel to `entries`); the caller pre-creates each AS via
    /// [`crate::device::RhiDevice::create_acceleration_structure`].
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan backend
    /// overrides it under `feature="hwrt"`.
    #[cold]
    #[inline(never)]
    fn cmd_build_acceleration_structures(
        &mut self,
        _entries: &[AsBuildEntry],
        _dest: &[&A::AccelerationStructure],
    ) {
        // HW-RT R2a-1 default seam: a backend without an AS path leaves this a no-op; the
        // Vulkan backend overrides it under `feature="hwrt"`.
    }

    /// Records the `ACCELERATION_STRUCTURE_BUILD → *` execution/memory barrier
    /// (`vkCmdPipelineBarrier` with
    /// `ACCELERATION_STRUCTURE_WRITE_BIT_KHR → ACCELERATION_STRUCTURE_READ_BIT_KHR`,
    /// HW-RT rung R2a-1). Ordered after a build so a subsequent read (the `rayQuery`
    /// resolve at R2a-4, or a dependent build) observes the finished structure.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]`; the Vulkan backend
    /// overrides it under `feature="hwrt"`.
    #[cold]
    #[inline(never)]
    fn cmd_acceleration_structure_barrier(&mut self) {
        // HW-RT R2a-1 default seam: a backend without an AS path leaves this a no-op; the
        // Vulkan backend overrides it under `feature="hwrt"`.
    }
}
