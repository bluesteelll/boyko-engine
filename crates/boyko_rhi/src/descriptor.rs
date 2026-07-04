//! POD descriptor structs passed into the RHI create/record calls.
//!
//! Each foundation descriptor fits within a cache line; barriers are stack
//! locals the backend walks once. Recording is single-threaded on the dispatcher
//! during the apply-window (plan §5.3), so false-sharing is not a concern.

use crate::api::RhiApi;
use crate::enums::{
    BarrierAccess, BarrierStage, BlendState, BufferUsage, CullMode, Format, ImageAspect,
    ImageLayout, LoadOp, MemoryLocation, PrimitiveTopology, StoreOp, VertexFormat,
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

/// Parameters for [`crate::device::RhiDevice::create_query_pool`] (HW-RT rung R0).
///
/// `#[repr(C)]` POD (a single `u32` count) so the field layout is stable for a
/// backend to read — it maps onto a `VkQueryPoolCreateInfo` with `queryType =
/// TIMESTAMP`, `queryCount = count`, `pipelineStatistics = 0`. A TIMESTAMP query pool
/// is UNDEFINED at creation; the caller MUST reset every query
/// ([`crate::encoder::RhiCommandEncoder::reset_query_pool`]) before its first
/// [`crate::encoder::RhiCommandEncoder::write_timestamp`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryPoolDesc {
    /// The number of timestamp queries the pool holds. The bracket collector sizes it
    /// to `2 * PASS_COUNT` (a begin/end pair per timed pass).
    pub count: u32,
}

// ===========================================================================
// HW-RT rung R2a-1 — acceleration-structure POD descriptors.
//
// Agnostic carriers (plan D5 seam discipline: NO `Vk*` leak into `boyko_rhi`).
// Device/scratch addresses are plain `u64` device addresses (the backend obtains
// them via `get_buffer_device_address` / `get_acceleration_structure_device_address`).
// R2a-1 defines the vocabulary; R2a-2 (BLAS build) + R2a-3 (per-frame TLAS) consume
// it. Nothing here references a backend type, so the whole block is UNGATED — it
// carries no FFI and compiles in every build (byte-identical: no consumer calls the
// AS verbs when `hwrt` is OFF, they are `#[cold]` erroring defaults).
// ===========================================================================

/// Which acceleration-structure level an [`AsBuildEntry`] targets (HW-RT rung R2a-1):
/// a bottom-level (per-mesh triangle geometry) or a top-level (an instance array over
/// BLASes). Maps to `VkAccelerationStructureTypeKHR` backend-side.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsKind {
    /// A bottom-level acceleration structure (triangle geometry).
    /// `VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR`.
    Blas = 0,
    /// A top-level acceleration structure (an instance array). `VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR`.
    Tlas = 1,
}

/// One triangle-geometry (or instance-array) input to a BLAS/TLAS build
/// (HW-RT rung R2a-1). A `#[repr(C)]` POD carrier of the *device addresses* + counts
/// the backend needs to fill a `VkAccelerationStructureGeometryKHR` — no backend
/// handle, no `Vk*` type. For a TLAS this describes the instance array
/// (`vertex_data` = the `VkAccelerationStructureInstanceKHR[]` device address,
/// `primitive_count` = the instance count); the triangle fields are unread there.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsGeometryDesc {
    /// Device address of the vertex buffer (BLAS) or the instance array (TLAS).
    pub vertex_data: u64,
    /// Device address of the index buffer (BLAS only; `0` for a TLAS).
    pub index_data: u64,
    /// Byte stride between consecutive vertices (BLAS only).
    pub vertex_stride: u64,
    /// The highest vertex index referenced (`vertexCount - 1`, BLAS only).
    pub max_vertex: u32,
    /// The number of primitives: triangles (`indexCount / 3`, BLAS) or instances (TLAS).
    pub primitive_count: u32,
}

/// The build-scratch + result sizes a `vkGetAccelerationStructureBuildSizesKHR`
/// query returns for one build (HW-RT rung R2a-1). A `#[repr(C)]` POD echo of
/// `VkAccelerationStructureBuildSizesInfoKHR` in agnostic bytes: the caller sizes
/// the AS-backing buffer to `as_size` and the scratch buffer to `build_scratch`
/// (aligned to `DeviceCaps::as_scratch_align`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AsBuildSizes {
    /// Bytes the acceleration-structure backing buffer must hold.
    pub as_size: u64,
    /// Bytes the scratch buffer must hold for a fresh build.
    pub build_scratch: u64,
    /// Bytes the scratch buffer must hold for an in-place update (refit; `0` when
    /// the build was not created with `ALLOW_UPDATE`). R2a rebuilds every frame, so
    /// this is recorded but unused until R6.
    pub update_scratch: u64,
}

/// One acceleration-structure build entry recorded by
/// [`crate::encoder::RhiCommandEncoder::cmd_build_acceleration_structures`]
/// (HW-RT rung R2a-1). A `#[repr(C)]` POD carrier: the target level, its geometry,
/// the destination AS device address, and the scratch device address — everything
/// the backend needs to fill a `VkAccelerationStructureBuildGeometryInfoKHR` +
/// `VkAccelerationStructureBuildRangeInfoKHR` without a backend handle in the struct.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsBuildEntry {
    /// Whether this builds a BLAS or a TLAS.
    pub kind: AsKind,
    /// The geometry (BLAS triangles) or instance array (TLAS) to build from.
    pub geometry: AsGeometryDesc,
    /// Device address of the scratch buffer (aligned to `DeviceCaps::as_scratch_align`).
    pub scratch_address: u64,
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
/// module + entry name (+ the optional bind-group layout) for the duration of the
/// create call (the backend copies what it needs; nothing is retained past the
/// call).
pub struct ComputePipelineDesc<'a, A: RhiApi> {
    /// The compiled shader module the pipeline's compute stage is built from.
    pub module: &'a A::ShaderModule,
    /// The shader entry-point name (today always `c"main"`).
    pub entry: &'a core::ffi::CStr,
    /// Size in bytes of the push-constant range bound at pipeline-layout time
    /// (4, today — the foundation's single u32 push constant).
    pub push_constant_bytes: u32,
    /// The bind-group layout the pipeline layout includes at `set 0` (Render P1a:
    /// the multi-resource vocabulary set — e.g. a storage buffer + a storage image),
    /// or `None` to keep the device-shared **fixed** single-STORAGE_BUFFER compute
    /// layout (the packed-buffer offscreen path). `Some(layout)` builds a dedicated
    /// pipeline layout declaring that set + the push range, so a
    /// [`crate::encoder::RhiCommandEncoder::bind_descriptor_set_compute`] can bind a
    /// matching group before the dispatch; mirrors the graphics path's optional
    /// [`GraphicsPipelineDesc::bind_group_layout`].
    ///
    /// **Push-constant note (review O1):** a `Some(layout)` (vocabulary) pipeline gets
    /// a DEDICATED pipeline layout that is NOT push-constant-compatible with the
    /// device-shared compute layout. The current
    /// [`crate::encoder::RhiCommandEncoder::push_constants`] records against that
    /// shared layout, so it is valid only for the `None` (fixed-layout) path — a push
    /// while a vocabulary pipeline is bound is a Vulkan validation error. A vocabulary
    /// pipeline must push against its own layout (P1b adds a pipeline-scoped
    /// `push_constants` variant for the marcher's camera block).
    pub bind_group_layout: Option<&'a A::BindGroupLayout>,
}

/// One vertex attribute within a [`VertexBufferLayout`] (Phase-6 S0 rung 3).
///
/// `#[repr(C)]` so the field layout is stable for a backend to read. Maps onto a
/// `VkVertexInputAttributeDescription`'s `(location, format, offset)` — the
/// `binding` is supplied by the enclosing [`VertexBufferLayout`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttribute {
    /// The shader input location (`layout(location = N)` / the HLSL semantic slot).
    pub location: u32,
    /// The attribute's byte offset within a single vertex.
    pub offset: u32,
    /// The attribute's component format.
    pub format: VertexFormat,
}

/// A single vertex buffer's layout for a [`GraphicsPipelineDesc`] (Phase-6 S0 rung
/// 3): the per-vertex stride + the attributes packed within each vertex.
///
/// Maps onto one `VkVertexInputBindingDescription` (binding `0`, per-vertex input
/// rate) plus one `VkVertexInputAttributeDescription` per `attributes` entry. The
/// `'a` lifetime borrows the attribute slice for the `create_graphics_pipeline`
/// call only (the backend copies what it needs).
pub struct VertexBufferLayout<'a> {
    /// The byte stride between consecutive vertices in the buffer.
    pub stride: u32,
    /// The attributes packed within each vertex (rung 3: position + color).
    pub attributes: &'a [VertexAttribute],
}

/// Depth-bias (polygon-offset) state for a graphics pipeline (CSM Increment 0).
///
/// `#[repr(C)]` POD with an explicit field order so a backend reads it without
/// depending on Rust's default field reordering — it lowers onto a
/// `VkPipelineRasterizationStateCreateInfo`'s `(depthBiasConstantFactor,
/// depthBiasSlopeFactor, depthBiasClamp)` (with `depthBiasEnable = VK_TRUE`).
/// Carried as `Option<DepthBias>` on [`GraphicsPipelineDesc::depth_bias`]: `None`
/// keeps `depthBiasEnable = VK_FALSE` for every existing pipeline (byte-identical to
/// today); `Some(b)` enables the offset, which a shadow-map depth pass uses to push
/// occluder depth away from the light and kill shadow acne.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthBias {
    /// The constant depth offset added to every fragment (in depth-format units).
    pub constant_factor: f32,
    /// The offset scaled by the fragment's maximum depth slope.
    pub slope_factor: f32,
    /// The maximum (or minimum, if negative) depth bias applied; `0.0` disables the
    /// clamp.
    pub clamp: f32,
}

/// Parameters for [`crate::device::RhiDevice::create_graphics_pipeline`]
/// (Phase-6 S0 rung 2: a Vulkan 1.3 dynamic-rendering graphics pipeline).
///
/// Generic over the backend `A` because it borrows that backend's owned vertex +
/// fragment shader modules by reference; the `'a` lifetime ties the descriptor to
/// the borrowed modules + entry names + the vertex layout for the create call (the
/// backend copies what it needs; nothing is retained past the call).
///
/// Rung 3 adds a real **vertex input** layout (`vertex_layout`: binding stride +
/// position/color attributes) and a single **push-constant range** (`push_constant_
/// bytes`, a `VERTEX`-stage MVP `float4x4` = 64 bytes); a rung-3 pipeline still binds
/// **no** descriptor sets (the layout has only the push range). A `None`
/// `vertex_layout` keeps the rung-2 vertex-less (`SV_VertexID`) path, and `0`
/// `push_constant_bytes` keeps the rung-2 empty layout — so the rung-2 triangle test
/// stays valid against the same descriptor.
///
/// Rung 6 turns `color_formats` into a **slice** so a single pipeline can declare
/// **N color attachments** — the deferred-shading G-buffer's multiple render targets
/// (MRT). The backend builds one `VkPipelineColorBlendAttachmentState` per element
/// and an N-format `VkPipelineRenderingCreateInfo` array. A **one-element** slice
/// (e.g. `&[Format::R8G8B8A8Unorm]`) is the no-MRT path rungs 2..5 use; the geometry
/// G-buffer pass passes two (albedo + normal). The slice must be non-empty.
///
/// **Format contract (S0 SAFETY obligation, W2-b):** `color_formats[i]` MUST equal
/// the format of color attachment `i` of every [`RenderingDesc`] this pipeline is
/// bound inside — including the **count** (the pipeline's attachment count must equal
/// the rendering scope's) — the `VkPipelineRenderingCreateInfo` attachment formats
/// must match the dynamic-rendering scope, or the validation layer faults at **draw**
/// time, not at create time. The contract is documented on the Vulkan creation block
/// and re-stated on [`RenderingAttachment`].
pub struct GraphicsPipelineDesc<'a, A: RhiApi> {
    /// The compiled vertex-stage shader module.
    pub vertex_module: &'a A::ShaderModule,
    /// The vertex-stage entry-point name (today `c"main"`).
    pub vertex_entry: &'a core::ffi::CStr,
    /// The compiled fragment-stage shader module.
    pub fragment_module: &'a A::ShaderModule,
    /// The fragment-stage entry-point name (today `c"main"`).
    pub fragment_entry: &'a core::ffi::CStr,
    /// The color attachments' formats, one per MRT target (the
    /// `VkPipelineRenderingCreateInfo` formats — see the format contract above). A
    /// one-element slice is the no-MRT path; the G-buffer geometry pass passes two
    /// (albedo + normal). Borrowed for the `create_graphics_pipeline` call only.
    ///
    /// **EMPTY** is the DEPTH-ONLY path (CSM Increment 0): a `&[]` slice builds a
    /// pipeline with `colorAttachmentCount = 0` (null color-blend + null
    /// `pColorAttachmentFormats`) — a depth-only shadow-map pass that writes depth
    /// only. A depth-only pipeline REQUIRES a `depth_format` (validation rejects a
    /// pipeline with neither color nor depth).
    pub color_formats: &'a [Format],
    /// The depth attachment's format, or `None` for a depth-less pipeline (rungs
    /// 1..3). `Some(fmt)` (rung 4: [`Format::D32Sfloat`]) enables the pipeline's
    /// depth-stencil state (`depthTestEnable`/`depthWriteEnable`, compare op
    /// `LESS`, no stencil) and sets `VkPipelineRenderingCreateInfo.depthAttachmentFormat`
    /// — so the format MUST equal every [`DepthAttachment`]'s texture format (W2-b).
    pub depth_format: Option<Format>,
    /// The primitive-assembly topology (rung 2/3: `TriangleList`).
    pub topology: PrimitiveTopology,
    /// The vertex-buffer input layout, or `None` for a vertex-buffer-less pipeline
    /// (the rung-2 `SV_VertexID`-generated triangle).
    pub vertex_layout: Option<VertexBufferLayout<'a>>,
    /// The `VERTEX`-stage push-constant range size in bytes at offset `0` (rung 3:
    /// `64` for an MVP `float4x4`); `0` builds an empty pipeline layout (rung 2).
    pub push_constant_bytes: u32,
    /// The bind-group layout the pipeline layout includes at `set 0` (Phase-6 S0
    /// rung 5: one COMBINED_IMAGE_SAMPLER for the sampled texture), or `None` for a
    /// descriptor-less pipeline (rungs 1..4). `Some(layout)` makes the pipeline
    /// layout declare that set, so a [`crate::encoder::RhiCommandEncoder::bind_descriptor_set`]
    /// can bind a matching bind group before the sampling draw; `None` keeps the
    /// rungs-1..4 no-descriptor path valid (empty / push-only pipeline layout).
    pub bind_group_layout: Option<&'a A::BindGroupLayout>,
    /// The color-blend state for the color attachment(s) (GUI P5a Decision 3), or
    /// `None` for the engine's default opaque (blend-disabled) write. `None` is
    /// byte-identical to the pre-P5a behavior, so every existing pipeline passes
    /// `None`; the UI pipeline passes `Some(BlendState::PREMULTIPLIED_ALPHA)`. A
    /// `Some(bs)` is lowered onto the same blend factors/op for ALL color
    /// attachments (P5a UI is single-target; per-target MRT blend is a future
    /// widening of `Option<BlendState>` → a per-target slice).
    pub blend: Option<BlendState>,
    /// The triangle face-culling mode (CSM Increment 0). [`CullMode::None`] (the
    /// default) is byte-identical to today's hardcoded `VK_CULL_MODE_NONE`, so every
    /// existing pipeline re-emits an identical rasterization state; a shadow-map depth
    /// pass selects [`CullMode::Front`].
    pub cull_mode: CullMode,
    /// The optional depth-bias (polygon-offset) state (CSM Increment 0). `None` (the
    /// default) keeps `depthBiasEnable = VK_FALSE` — byte-identical to today; `Some(b)`
    /// enables the offset for a shadow-map depth pass (kills shadow acne).
    pub depth_bias: Option<DepthBias>,
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
/// format MUST equal the format declared at `create_graphics_pipeline` time for the
/// same color-attachment index, for any pipeline bound inside this rendering scope —
/// and the rendering scope's attachment **count** must equal the bound pipeline's
/// `color_formats` length (rung 6 MRT) — a mismatch faults at draw time in the
/// validation layer, not at create time. Rung 1 binds no pipeline, so the contract
/// is vacuously satisfied; it is documented here for the later rung.
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

/// The optional depth attachment for [`crate::encoder::RhiCommandEncoder::begin_rendering`]
/// (Phase-6 S0 rung 4 — `VkRenderingInfo.pDepthAttachment`).
///
/// Borrows the backend texture (whose DEPTH-aspect image view is the attachment)
/// for the `begin_rendering` call only. `clear_depth` is applied iff `load_op` is
/// [`LoadOp::Clear`] (rung 4 clears to `1.0`, the far plane, so a smaller fragment
/// `z` always wins the `LESS` test).
///
/// **Format contract (S0 SAFETY obligation, W2-b):** the attachment's texture
/// format MUST equal the depth format declared at `create_graphics_pipeline` time
/// (`GraphicsPipelineDesc::depth_format`) for any pipeline bound inside this
/// rendering scope — a mismatch faults at draw time in the validation layer.
pub struct DepthAttachment<'a, A: RhiApi> {
    /// The texture whose DEPTH-aspect image view is bound as the depth attachment.
    pub texture: &'a A::Texture,
    /// The layout the depth attachment is in during rendering (typically
    /// [`ImageLayout::DepthAttachmentOptimal`]).
    pub layout: ImageLayout,
    /// What to do with the depth contents at scope entry (rung 4: [`LoadOp::Clear`]).
    pub load_op: LoadOp,
    /// What to do with the rendered depth at scope exit (rung 4: [`StoreOp::Store`]).
    pub store_op: StoreOp,
    /// The depth clear value used when `load_op == LoadOp::Clear` (rung 4: `1.0`).
    pub clear_depth: f32,
}

/// Parameters for [`crate::encoder::RhiCommandEncoder::begin_rendering`]
/// (Vulkan 1.3 dynamic rendering — no `VkRenderPass`/`VkFramebuffer`).
///
/// The `colors` slice is a stack local the backend walks once; rung 1 supplies
/// exactly one color attachment and no depth attachment. Rung 4 adds an optional
/// `depth` attachment (`None` keeps the rungs-1..3 no-depth path valid). Rung 6 binds
/// N color attachments (the G-buffer MRT) — the count must equal the bound pipeline's
/// `color_formats` length (W2-b).
pub struct RenderingDesc<'a, A: RhiApi> {
    /// The region the rendering scope covers.
    pub render_area: RenderArea,
    /// The color attachments bound for this scope (rung 1: exactly one; rung 6: the
    /// G-buffer's N targets, in the same order as the pipeline's `color_formats`).
    pub colors: &'a [RenderingAttachment<'a, A>],
    /// The optional depth attachment (Phase-6 S0 rung 4); `None` for the no-depth
    /// rungs-1..3 path.
    pub depth: Option<DepthAttachment<'a, A>>,
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

    /// The full single-mip, single-layer DEPTH range — the rung-4 depth-attachment
    /// barrier default (the depth counterpart of [`Self::COLOR`]).
    pub const DEPTH: ImageSubresourceRange = ImageSubresourceRange {
        aspect: ImageAspect::DEPTH,
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
