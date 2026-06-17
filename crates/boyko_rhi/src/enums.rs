//! Thin, backend-agnostic enums and bitflags consumed by the RHI descriptors.
//!
//! Per plan D5 these define **only** what the headless-compute foundation uses.
//! The bitflag families ([`BufferUsage`], [`ShaderStage`], [`BarrierStage`],
//! [`BarrierAccess`]) deliberately pick **numeric values equal to the
//! corresponding Vulkan `VkFlags` (u32) constants** so a Vulkan backend's
//! `to_vk` is a zero-cost identity cast (`#[inline]`). A backend whose native
//! constants differ (DX12/Metal) translates with a `match` at the cold
//! resource-create boundary — never in a hot loop.
//!
//! `boyko_rhi` does **not** depend on the Vulkan crate; it merely chooses
//! matching numeric values. `Format`/image-layout are `i32` in the FFI (not u32
//! bitflags) and are **not** needed by the compute path, so they are a Phase-2-3
//! seam (plan W1) and intentionally absent here.

use core::ops::BitOr;

/// Buffer usage bitflags. Values equal the `VK_BUFFER_USAGE_*` bits so a Vulkan
/// `to_vk` is an identity cast.
///
/// `#[repr(transparent)]` over a `u32` so it shares the layout of the underlying
/// `VkFlags` and the cast is a no-op.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferUsage(u32);

impl BufferUsage {
    /// `VK_BUFFER_USAGE_TRANSFER_SRC_BIT`.
    pub const TRANSFER_SRC: BufferUsage = BufferUsage(0x0000_0001);
    /// `VK_BUFFER_USAGE_TRANSFER_DST_BIT`.
    pub const TRANSFER_DST: BufferUsage = BufferUsage(0x0000_0002);
    /// `VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT`.
    pub const UNIFORM: BufferUsage = BufferUsage(0x0000_0010);
    /// `VK_BUFFER_USAGE_STORAGE_BUFFER_BIT`.
    pub const STORAGE: BufferUsage = BufferUsage(0x0000_0020);
    /// `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT`.
    pub const INDIRECT: BufferUsage = BufferUsage(0x0000_0100);
    /// `VK_BUFFER_USAGE_VERTEX_BUFFER_BIT` (Phase-6 S0 rung 3).
    pub const VERTEX: BufferUsage = BufferUsage(0x0000_0080);
    /// `VK_BUFFER_USAGE_INDEX_BUFFER_BIT` (Phase-6 S0 rung 3 seam; rung 3 draws
    /// non-indexed, so this is defined for the `bind_index_buffer` verb but unused).
    pub const INDEX: BufferUsage = BufferUsage(0x0000_0040);

    /// The empty set (no usage bits).
    pub const NONE: BufferUsage = BufferUsage(0);

    /// Returns the raw `u32` bit pattern — equal to the Vulkan `VkFlags` value.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns `true` if every bit in `other` is also set in `self`.
    #[inline]
    pub const fn contains(self, other: BufferUsage) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl BitOr for BufferUsage {
    type Output = BufferUsage;

    #[inline]
    fn bitor(self, rhs: BufferUsage) -> BufferUsage {
        BufferUsage(self.0 | rhs.0)
    }
}

/// Shader-stage bitflags. Values equal the `VK_SHADER_STAGE_*` bits.
///
/// `COMPUTE` is the only stage the foundation uses; `VERTEX`/`FRAGMENT` are
/// defined now for the Phase-6+ graphics seam so the trait surface stays stable.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShaderStage(u32);

impl ShaderStage {
    /// `VK_SHADER_STAGE_VERTEX_BIT` (seam: Phase 6+ graphics).
    pub const VERTEX: ShaderStage = ShaderStage(0x0000_0001);
    /// `VK_SHADER_STAGE_FRAGMENT_BIT` (seam: Phase 6+ graphics).
    pub const FRAGMENT: ShaderStage = ShaderStage(0x0000_0010);
    /// `VK_SHADER_STAGE_COMPUTE_BIT`.
    pub const COMPUTE: ShaderStage = ShaderStage(0x0000_0020);

    /// Returns the raw `u32` bit pattern — equal to the Vulkan `VkFlags` value.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for ShaderStage {
    type Output = ShaderStage;

    #[inline]
    fn bitor(self, rhs: ShaderStage) -> ShaderStage {
        ShaderStage(self.0 | rhs.0)
    }
}

/// Pipeline-stage bitflags for a [`crate::descriptor::BarrierDesc`]. Values equal
/// the `VK_PIPELINE_STAGE_*` bits.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BarrierStage(u32);

impl BarrierStage {
    /// `VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT`.
    pub const TOP_OF_PIPE: BarrierStage = BarrierStage(0x0000_0001);
    /// `VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`.
    pub const COMPUTE_SHADER: BarrierStage = BarrierStage(0x0000_0800);
    /// `VK_PIPELINE_STAGE_TRANSFER_BIT`.
    pub const TRANSFER: BarrierStage = BarrierStage(0x0000_1000);
    /// `VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT` (graphics seam: Phase 6+).
    pub const COLOR_ATTACHMENT_OUTPUT: BarrierStage = BarrierStage(0x0000_0400);
    /// `VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT`.
    pub const BOTTOM_OF_PIPE: BarrierStage = BarrierStage(0x0000_2000);

    /// The empty set (no stage bits) — an invalid barrier; callers must set at
    /// least one when a buffer barrier is present (asserted at the encoder).
    pub const NONE: BarrierStage = BarrierStage(0);

    /// Returns the raw `u32` bit pattern — equal to the Vulkan `VkFlags` value.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// `true` if no stage bit is set.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for BarrierStage {
    type Output = BarrierStage;

    #[inline]
    fn bitor(self, rhs: BarrierStage) -> BarrierStage {
        BarrierStage(self.0 | rhs.0)
    }
}

/// Memory-access bitflags for a [`crate::descriptor::BufferBarrier`]. Values
/// equal the `VK_ACCESS_*` bits.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BarrierAccess(u32);

impl BarrierAccess {
    /// `VK_ACCESS_SHADER_READ_BIT`.
    pub const SHADER_READ: BarrierAccess = BarrierAccess(0x0000_0020);
    /// `VK_ACCESS_SHADER_WRITE_BIT`.
    pub const SHADER_WRITE: BarrierAccess = BarrierAccess(0x0000_0040);
    /// `VK_ACCESS_TRANSFER_READ_BIT`.
    pub const TRANSFER_READ: BarrierAccess = BarrierAccess(0x0000_0800);
    /// `VK_ACCESS_TRANSFER_WRITE_BIT`.
    pub const TRANSFER_WRITE: BarrierAccess = BarrierAccess(0x0000_1000);
    /// `VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT` (graphics seam: Phase 6+).
    pub const COLOR_ATTACHMENT_WRITE: BarrierAccess = BarrierAccess(0x0000_0100);

    /// The empty set (no access bits).
    pub const NONE: BarrierAccess = BarrierAccess(0);

    /// Returns the raw `u32` bit pattern — equal to the Vulkan `VkFlags` value.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for BarrierAccess {
    type Output = BarrierAccess;

    #[inline]
    fn bitor(self, rhs: BarrierAccess) -> BarrierAccess {
        BarrierAccess(self.0 | rhs.0)
    }
}

/// Where a buffer's backing memory lives.
///
/// `HostVisibleCoherent` is the only location the headless-compute foundation
/// uses (one persistently-mapped host-coherent block). `DeviceLocal` is defined
/// for the Phase-5 `GpuColumn` staging seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryLocation {
    /// Host-visible + host-coherent: CPU-mappable, writes are visible to the GPU
    /// without an explicit flush. The foundation's single block.
    HostVisibleCoherent,
    /// Device-local: GPU-fast, not CPU-mappable; needs staging + flush (Phase 5).
    DeviceLocal,
}

// ===========================================================================
// Phase-6 graphics-surface enums (S0). Per plan §S0 / D5/W1: `Format` and
// `ImageLayout` are the FFI's `i32` (`VkFormat`/`VkImageLayout`) family — NOT
// identity-cast u32 bitflags — so they are modeled as `#[repr(i32)]` enums whose
// discriminants equal the Vulkan constants and which a backend maps with a cold
// `match` both directions. `ImageUsage` is a u32 bitflag family whose values
// equal the `VK_IMAGE_USAGE_*` bits (identity cast), mirroring `BufferUsage`.
// ===========================================================================

/// A pixel/texel format (the `VkFormat` `i32` family, plan D5/W1).
///
/// `#[repr(i32)]` with discriminants equal to the corresponding `VkFormat`
/// constants. A backend translates with a cold `match` at the resource-create
/// boundary (the equality is asserted backend-side in `abi_guard.rs`). Only the
/// formats the basic slice needs are defined; the family grows per phase.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// `VK_FORMAT_UNDEFINED` — no/unspecified format.
    Undefined = 0,
    /// `VK_FORMAT_R8G8B8A8_UNORM` — the rung-1 offscreen golden-readback format.
    R8G8B8A8Unorm = 37,
    /// `VK_FORMAT_B8G8R8A8_UNORM` — the common swapchain format.
    B8G8R8A8Unorm = 44,
    /// `VK_FORMAT_R16_SFLOAT` — a compact single-channel float (deferred SDF use).
    R16Sfloat = 76,
    /// `VK_FORMAT_R32G32B32_SFLOAT` — three 32-bit floats (deferred position use).
    R32G32B32Sfloat = 106,
    /// `VK_FORMAT_D32_SFLOAT` — a 32-bit float depth attachment (deferred S1 use).
    D32Sfloat = 126,
}

impl Format {
    /// The raw `i32` discriminant — equal to the matching `VkFormat` constant.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// An image layout (the `VkImageLayout` `i32` family, plan D5/W1).
///
/// `#[repr(i32)]`; discriminants equal the matching `VkImageLayout` constants. A
/// backend maps with a cold `match` (asserted backend-side). Only the layouts
/// rung-1 needs (the clear → readback transitions) are defined.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageLayout {
    /// `VK_IMAGE_LAYOUT_UNDEFINED` — contents undefined; the only valid `old`
    /// layout for the first transition of a fresh image.
    Undefined = 0,
    /// `VK_IMAGE_LAYOUT_GENERAL` — usable by any access (e.g. a compute store).
    General = 1,
    /// `VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL` — bound as a color attachment.
    ColorAttachmentOptimal = 2,
    /// `VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL` — a transfer/copy source.
    TransferSrcOptimal = 6,
    /// `VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL` — a transfer/copy destination.
    TransferDstOptimal = 7,
}

impl ImageLayout {
    /// The raw `i32` discriminant — equal to the matching `VkImageLayout`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Image usage bitflags. Values equal the `VK_IMAGE_USAGE_*` bits so a Vulkan
/// `to_vk` is an identity cast (mirroring [`BufferUsage`], plan OQ-5/W2-c).
///
/// `#[repr(transparent)]` over a `u32`. The G-buffer reserves both `STORAGE`
/// (compute sphere-trace, OQ-5) and `COLOR_ATTACHMENT` (mesh raster) usage; rung
/// 1 itself only needs `COLOR_ATTACHMENT | TRANSFER_SRC` (clear → readback) and
/// `STORAGE` (the D3 storage image of a later rung).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageUsage(u32);

impl ImageUsage {
    /// `VK_IMAGE_USAGE_TRANSFER_SRC_BIT`.
    pub const TRANSFER_SRC: ImageUsage = ImageUsage(0x0000_0001);
    /// `VK_IMAGE_USAGE_TRANSFER_DST_BIT`.
    pub const TRANSFER_DST: ImageUsage = ImageUsage(0x0000_0002);
    /// `VK_IMAGE_USAGE_SAMPLED_BIT`.
    pub const SAMPLED: ImageUsage = ImageUsage(0x0000_0004);
    /// `VK_IMAGE_USAGE_STORAGE_BIT`.
    pub const STORAGE: ImageUsage = ImageUsage(0x0000_0008);
    /// `VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT`.
    pub const COLOR_ATTACHMENT: ImageUsage = ImageUsage(0x0000_0010);

    /// The empty set (no usage bits).
    pub const NONE: ImageUsage = ImageUsage(0);

    /// Returns the raw `u32` bit pattern — equal to the Vulkan `VkFlags` value.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns `true` if every bit in `other` is also set in `self`.
    #[inline]
    pub const fn contains(self, other: ImageUsage) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl BitOr for ImageUsage {
    type Output = ImageUsage;

    #[inline]
    fn bitor(self, rhs: ImageUsage) -> ImageUsage {
        ImageUsage(self.0 | rhs.0)
    }
}

/// The dimensionality of a texture (`VkImageType` family).
///
/// `#[repr(i32)]`; discriminants equal `VkImageType`. Rung 1 only creates `D2`
/// color images; `D3` is reserved for the deferred SDF storage image.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureDimension {
    /// `VK_IMAGE_TYPE_2D`.
    D2 = 1,
    /// `VK_IMAGE_TYPE_3D` (deferred SDF storage image).
    D3 = 2,
}

impl TextureDimension {
    /// The raw `i32` discriminant — equal to the matching `VkImageType`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A dynamic-rendering attachment load op (`VkAttachmentLoadOp` family).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadOp {
    /// `VK_ATTACHMENT_LOAD_OP_LOAD` — preserve existing contents.
    Load = 0,
    /// `VK_ATTACHMENT_LOAD_OP_CLEAR` — clear to the attachment's clear value.
    Clear = 1,
    /// `VK_ATTACHMENT_LOAD_OP_DONT_CARE` — contents are undefined.
    DontCare = 2,
}

impl LoadOp {
    /// The raw `i32` discriminant — equal to the matching `VkAttachmentLoadOp`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A dynamic-rendering attachment store op (`VkAttachmentStoreOp` family).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreOp {
    /// `VK_ATTACHMENT_STORE_OP_STORE` — write the result back to the attachment.
    Store = 0,
    /// `VK_ATTACHMENT_STORE_OP_DONT_CARE` — the result may be discarded.
    DontCare = 1,
}

impl StoreOp {
    /// The raw `i32` discriminant — equal to the matching `VkAttachmentStoreOp`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// The primitive-assembly topology a graphics pipeline rasterizes
/// (`VkPrimitiveTopology` family, Phase-6 S0 rung 2).
///
/// `#[repr(i32)]`; discriminants equal the matching `VkPrimitiveTopology`
/// constants (asserted backend-side). Rung 2 draws a single `TriangleList`;
/// the family grows per phase.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTopology {
    /// `VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST` — every three vertices a triangle.
    TriangleList = 3,
}

impl PrimitiveTopology {
    /// The raw `i32` discriminant — equal to the matching `VkPrimitiveTopology`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// The format of a single vertex attribute (`VkFormat` family, Phase-6 S0 rung 3).
///
/// `#[repr(i32)]` with discriminants equal to the matching `VkFormat` constants
/// (asserted backend-side). Only the attribute formats rung 3 needs (a 3-float
/// position + a 4-float color) are defined; the family grows per phase. This is a
/// distinct enum from [`Format`] so the vertex-attribute set stays a small,
/// purpose-scoped vocabulary, but the discriminants share the `VkFormat` space.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexFormat {
    /// `VK_FORMAT_R32G32B32_SFLOAT` — three 32-bit floats (a vec3 position).
    Float32x3 = 106,
    /// `VK_FORMAT_R32G32B32A32_SFLOAT` — four 32-bit floats (a vec4 color).
    Float32x4 = 109,
}

impl VertexFormat {
    /// The raw `i32` discriminant — equal to the matching `VkFormat` constant.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// The index width for [`crate::encoder::RhiCommandEncoder::bind_index_buffer`]
/// (`VkIndexType` family, Phase-6 S0 rung-3 seam).
///
/// `#[repr(i32)]`; discriminants equal the matching `VkIndexType` constants. Rung 3
/// draws non-indexed, so this exists only so the `bind_index_buffer` verb has a
/// stable, typed width argument for the later rung that uses indices.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexType {
    /// `VK_INDEX_TYPE_UINT16`.
    Uint16 = 0,
    /// `VK_INDEX_TYPE_UINT32`.
    Uint32 = 1,
}

impl IndexType {
    /// The raw `i32` discriminant — equal to the matching `VkIndexType`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Image-aspect bitflags for an image subresource. Values equal the
/// `VK_IMAGE_ASPECT_*` bits.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageAspect(u32);

impl ImageAspect {
    /// `VK_IMAGE_ASPECT_COLOR_BIT`.
    pub const COLOR: ImageAspect = ImageAspect(0x0000_0001);
    /// `VK_IMAGE_ASPECT_DEPTH_BIT` (deferred depth-attachment use).
    pub const DEPTH: ImageAspect = ImageAspect(0x0000_0002);

    /// Returns the raw `u32` bit pattern — equal to the Vulkan `VkFlags` value.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for ImageAspect {
    type Output = ImageAspect;

    #[inline]
    fn bitor(self, rhs: ImageAspect) -> ImageAspect {
        ImageAspect(self.0 | rhs.0)
    }
}
