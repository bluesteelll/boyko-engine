//! POD descriptor structs passed into the RHI create/record calls.
//!
//! Each foundation descriptor fits within a cache line; barriers are stack
//! locals the backend walks once. Recording is single-threaded on the dispatcher
//! during the apply-window (plan §5.3), so false-sharing is not a concern.

use crate::api::RhiApi;
use crate::enums::{
    BarrierAccess, BarrierStage, BufferUsage, Format, ImageAspect, ImageLayout, LoadOp,
    MemoryLocation, PrimitiveTopology, StoreOp,
};

/// Parameters for [`crate::device::RhiDevice::create_buffer`].
///
/// `#[repr(C)]` so the field layout is stable (size + usage + location) — a
/// backend can read it without depending on Rust's default field reordering.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferDesc {
    /// Size of the buffer in bytes.
    pub size: u64,
    /// Usage bits the buffer must support.
    pub usage: BufferUsage,
    /// Where the backing memory lives.
    pub location: MemoryLocation,
}

/// One buffer-to-buffer copy region for
/// [`crate::encoder::RhiCommandEncoder::copy_buffer`].
///
/// `#[repr(C)]` with the exact `(src_offset, dst_offset, size)` field order and
/// `u64` types of Vulkan's `VkBufferCopy`, so a Vulkan backend can reinterpret a
/// `&[BufferCopy]` as a `&[VkBufferCopy]` without a per-region copy (the layout
/// match is asserted backend-side, plan MF-8). Used for the Phase-5 staging
/// upload + the test-only readback.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferCopy {
    /// Byte offset within the source buffer.
    pub src_offset: u64,
    /// Byte offset within the destination buffer.
    pub dst_offset: u64,
    /// Number of bytes to copy.
    pub size: u64,
}

/// Parameters for [`crate::device::RhiDevice::create_compute_pipeline`].
///
/// Generic over the backend `A` because it borrows that backend's owned shader
/// module by reference. The `'a` lifetime ties the descriptor to the borrowed
/// module + entry name for the duration of the create call (the backend copies
/// what it needs; nothing is retained past the call).
pub struct ComputePipelineDesc<'a, A: RhiApi> {
    /// The compiled shader module the pipeline's compute stage is built from.
    pub module: &'a A::ShaderModule,
    /// The shader entry-point name (today always `c"main"`).
    pub entry: &'a core::ffi::CStr,
    /// Size in bytes of the push-constant range bound at pipeline-layout time
    /// (4, today — the foundation's single u32 push constant).
    pub push_constant_bytes: u32,
}

/// Parameters for [`crate::device::RhiDevice::create_graphics_pipeline`]
/// (Phase-6 S0 rung 2: a Vulkan 1.3 dynamic-rendering graphics pipeline).
///
/// Generic over the backend `A` because it borrows that backend's owned vertex +
/// fragment shader modules by reference; the `'a` lifetime ties the descriptor to
/// the borrowed modules + entry names for the create call (the backend copies what
/// it needs; nothing is retained past the call). Rung 2 binds **no** descriptor
/// sets (an empty pipeline layout) and **no** vertex buffer (the vertex shader
/// generates its positions from the vertex index), so the descriptor carries only
/// what the rasterizer + dynamic-rendering attachment-format chain needs.
///
/// **Format contract (S0 SAFETY obligation, W2-b):** `color_format` MUST equal the
/// format of every color attachment of any [`RenderingDesc`] this pipeline is bound
/// inside — the `VkPipelineRenderingCreateInfo` attachment format must match the
/// dynamic-rendering scope, or the validation layer faults at **draw** time, not at
/// create time. The contract is documented on the Vulkan creation block and
/// re-stated on [`RenderingAttachment`].
pub struct GraphicsPipelineDesc<'a, A: RhiApi> {
    /// The compiled vertex-stage shader module.
    pub vertex_module: &'a A::ShaderModule,
    /// The vertex-stage entry-point name (today `c"main"`).
    pub vertex_entry: &'a core::ffi::CStr,
    /// The compiled fragment-stage shader module.
    pub fragment_module: &'a A::ShaderModule,
    /// The fragment-stage entry-point name (today `c"main"`).
    pub fragment_entry: &'a core::ffi::CStr,
    /// The single color attachment's format (the
    /// `VkPipelineRenderingCreateInfo` format — see the format contract above).
    pub color_format: Format,
    /// The primitive-assembly topology (rung 2: `TriangleList`).
    pub topology: PrimitiveTopology,
}

/// A single buffer's access transition inside a [`BarrierDesc`].
///
/// `#[repr(C)]` for a stable, backend-readable layout. The `'a` lifetime borrows
/// the buffer for the barrier-record call only.
#[repr(C)]
pub struct BufferBarrier<'a, A: RhiApi> {
    /// The buffer whose access is transitioning.
    pub buffer: &'a A::Buffer,
    /// Access scope before the barrier (e.g. a prior shader write).
    pub src_access: BarrierAccess,
    /// Access scope after the barrier (e.g. a subsequent shader read).
    pub dst_access: BarrierAccess,
}

/// Parameters for [`crate::encoder::RhiCommandEncoder::pipeline_barrier`].
///
/// **Buffer-only** in Phase 1 (plan D3): the only image-layout transitions that
/// exist today live in the concrete `Renderer` and are not routed through the
/// trait. `ImageBarrier`/`images` is a genuine Phase-2-3 seam — intentionally
/// absent here.
///
/// The `buffers` slice is a stack local walked once by the backend; the
/// foundation chained-barrier path supplies 0 or 1 entries.
pub struct BarrierDesc<'a, A: RhiApi> {
    /// Pipeline stage(s) that must complete before the barrier.
    pub src_stage: BarrierStage,
    /// Pipeline stage(s) that wait on the barrier.
    pub dst_stage: BarrierStage,
    /// The buffer transitions covered by this barrier (foundation: 0 or 1).
    pub buffers: &'a [BufferBarrier<'a, A>],
}

// ===========================================================================
// Phase-6 graphics-surface descriptors (S0). The `image_barrier` + dynamic-
// rendering verbs the rung-1 offscreen-clear path needs, abstracting the
// concrete `swapchain.rs::record_clear` image-barrier + `VkRenderingInfo`
// pattern into the trait. Recording is single-threaded on the dispatcher in the
// apply-window (plan §5.3), so false sharing is not a concern.
// ===========================================================================

/// A texel-rectangle for a [`RenderingDesc`]'s render area (the `VkRect2D`
/// the dynamic-rendering scope covers).
///
/// `#[repr(C)]` mirroring `VkRect2D`'s `(offset, extent)` `i32 x,y` + `u32 w,h`
/// layout so a backend reads it without reordering.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderArea {
    /// X offset of the render area, in texels.
    pub x: i32,
    /// Y offset of the render area, in texels.
    pub y: i32,
    /// Width of the render area, in texels.
    pub width: u32,
    /// Height of the render area, in texels.
    pub height: u32,
}

/// A viewport for [`crate::encoder::RhiCommandEncoder::set_viewport`] (the dynamic
/// viewport state a graphics pipeline reads, Phase-6 S0 rung 2).
///
/// `#[repr(C)]` mirroring `VkViewport`'s `(x, y, width, height, min_depth,
/// max_depth)` `f32` layout so a backend reads it without reordering. Vulkan's
/// viewport y-axis points down; rung 2 uses the full surface with depth `[0, 1]`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// X offset of the viewport, in texels.
    pub x: f32,
    /// Y offset of the viewport, in texels.
    pub y: f32,
    /// Width of the viewport, in texels (`> 0`).
    pub width: f32,
    /// Height of the viewport, in texels (`> 0`).
    pub height: f32,
    /// Minimum depth (typically `0.0`).
    pub min_depth: f32,
    /// Maximum depth (typically `1.0`).
    pub max_depth: f32,
}

/// One color attachment for [`crate::encoder::RhiCommandEncoder::begin_rendering`].
///
/// Borrows the backend texture (whose image view is the attachment) for the
/// `begin_rendering` call only. `clear_color` is applied iff `load_op` is
/// [`LoadOp::Clear`].
///
/// **Format contract (S0 SAFETY obligation, W2-b):** the attachment's texture
/// format MUST equal the format declared at `create_graphics_pipeline` time for
/// any pipeline bound inside this rendering scope — a mismatch faults at draw
/// time in the validation layer, not at create time. Rung 1 binds no pipeline, so
/// the contract is vacuously satisfied; it is documented here for the later rung.
pub struct RenderingAttachment<'a, A: RhiApi> {
    /// The texture whose image view is bound as this color attachment.
    pub texture: &'a A::Texture,
    /// The layout the attachment is in during rendering (typically
    /// [`ImageLayout::ColorAttachmentOptimal`]).
    pub layout: ImageLayout,
    /// What to do with the attachment's existing contents at scope entry.
    pub load_op: LoadOp,
    /// What to do with the rendered result at scope exit.
    pub store_op: StoreOp,
    /// The RGBA clear value used when `load_op == LoadOp::Clear`.
    pub clear_color: [f32; 4],
}

/// Parameters for [`crate::encoder::RhiCommandEncoder::begin_rendering`]
/// (Vulkan 1.3 dynamic rendering — no `VkRenderPass`/`VkFramebuffer`).
///
/// The `colors` slice is a stack local the backend walks once; rung 1 supplies
/// exactly one color attachment and no depth attachment.
pub struct RenderingDesc<'a, A: RhiApi> {
    /// The region the rendering scope covers.
    pub render_area: RenderArea,
    /// The color attachments bound for this scope (rung 1: exactly one).
    pub colors: &'a [RenderingAttachment<'a, A>],
}

/// An image's `[base_mip, base_mip + level_count) × [base_layer, base_layer +
/// layer_count)` subresource range for an [`ImageBarrierDesc`].
///
/// `#[repr(C)]` mirroring `VkImageSubresourceRange`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSubresourceRange {
    /// Which aspect(s) of the image the range covers (color/depth).
    pub aspect: ImageAspect,
    /// First mip level in the range.
    pub base_mip_level: u32,
    /// Number of mip levels (`1` for a single-level image).
    pub level_count: u32,
    /// First array layer in the range.
    pub base_array_layer: u32,
    /// Number of array layers (`1` for a non-array image).
    pub layer_count: u32,
}

impl ImageSubresourceRange {
    /// The full single-mip, single-layer color range — the rung-1 default.
    pub const COLOR: ImageSubresourceRange = ImageSubresourceRange {
        aspect: ImageAspect::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
}

/// One image→buffer copy region for
/// [`crate::encoder::RhiCommandEncoder::copy_image_to_buffer`] (the S0 offscreen
/// golden-readback transfer).
///
/// `#[repr(C)]` mirroring `VkBufferImageCopy`'s `(buffer_offset,
/// buffer_row_length, buffer_image_height, image_subresource, image_offset,
/// image_extent)` layout so a backend reads it without reordering. The
/// `image_subresource` is flattened to its four scalar members and the
/// `image_offset`/`image_extent` to their scalar components, keeping this a
/// dependency-free POD on the `boyko_rhi` side (the backend asserts the layout
/// match against `VkBufferImageCopy`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferImageCopy {
    /// Byte offset within the destination buffer.
    pub buffer_offset: u64,
    /// Row length in texels (`0` = tightly packed to the image width).
    pub buffer_row_length: u32,
    /// Image height in texels (`0` = tightly packed to the image height).
    pub buffer_image_height: u32,
    /// Image-aspect bits the copy reads (typically [`ImageAspect::COLOR`]).
    pub aspect: ImageAspect,
    /// Mip level to copy from.
    pub mip_level: u32,
    /// First array layer to copy from.
    pub base_array_layer: u32,
    /// Number of array layers to copy (`1` for a non-array image).
    pub layer_count: u32,
    /// X texel offset of the copied region.
    pub image_offset_x: i32,
    /// Y texel offset of the copied region.
    pub image_offset_y: i32,
    /// Z texel offset of the copied region.
    pub image_offset_z: i32,
    /// Width of the copied region, in texels.
    pub image_extent_w: u32,
    /// Height of the copied region, in texels.
    pub image_extent_h: u32,
    /// Depth of the copied region, in texels (`1` for 2D).
    pub image_extent_d: u32,
}

/// Parameters for [`crate::encoder::RhiCommandEncoder::image_barrier`] (the
/// Phase-2-3 `ImageBarrier` seam, RHI plan D3/C1, now needed for S0).
///
/// One image-layout transition: old/new layout, the subresource range, and the
/// src/dst stage + access scopes. Borrows the backend texture for the record
/// call only. Abstracts the concrete UNDEFINED→COLOR / COLOR→TRANSFER_SRC
/// `VkImageMemoryBarrier`s of `swapchain.rs::record_clear`.
pub struct ImageBarrierDesc<'a, A: RhiApi> {
    /// The texture whose image's layout is transitioning.
    pub texture: &'a A::Texture,
    /// Pipeline stage(s) that must complete before the barrier.
    pub src_stage: BarrierStage,
    /// Pipeline stage(s) that wait on the barrier.
    pub dst_stage: BarrierStage,
    /// Access scope before the barrier.
    pub src_access: BarrierAccess,
    /// Access scope after the barrier.
    pub dst_access: BarrierAccess,
    /// The layout the image is in before the barrier.
    pub old_layout: ImageLayout,
    /// The layout the image is in after the barrier.
    pub new_layout: ImageLayout,
    /// The subresource range the transition covers.
    pub range: ImageSubresourceRange,
}
