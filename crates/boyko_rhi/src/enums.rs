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
    /// `VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT` — the fragment-shader stage that
    /// SAMPLES a texture (Phase-6 S0 rung 5: the COLOR_ATTACHMENT → SHADER_READ
    /// transition's destination stage, so the sampling draw waits on the prior
    /// pass's color write).
    pub const FRAGMENT_SHADER: BarrierStage = BarrierStage(0x0000_0080);
    /// `VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT` — the depth-test stage before
    /// the fragment shader (Phase-6 S0 rung 4: the depth-attachment barrier).
    pub const EARLY_FRAGMENT_TESTS: BarrierStage = BarrierStage(0x0000_0100);
    /// `VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT` — the depth-write stage after
    /// the fragment shader (Phase-6 S0 rung 4: the depth-attachment barrier).
    pub const LATE_FRAGMENT_TESTS: BarrierStage = BarrierStage(0x0000_0200);
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
    /// `VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT` (Phase-6 S0 rung 4).
    pub const DEPTH_STENCIL_ATTACHMENT_READ: BarrierAccess = BarrierAccess(0x0000_0200);
    /// `VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT` — the depth write the
    /// UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL barrier makes available (rung 4).
    pub const DEPTH_STENCIL_ATTACHMENT_WRITE: BarrierAccess = BarrierAccess(0x0000_0400);

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
    /// `VK_FORMAT_R8G8B8A8_SRGB` — sRGB-encoded 8-bit RGBA. Representable so a surface
    /// whose preferred swapchain format is sRGB can be named for the dynamic-rendering
    /// pipeline's color-attachment format match (the renderer otherwise skipped sRGB
    /// surfaces). The hardware applies linear→sRGB encoding on write to such an image.
    R8G8B8A8Srgb = 43,
    /// `VK_FORMAT_B8G8R8A8_UNORM` — the common swapchain format.
    B8G8R8A8Unorm = 44,
    /// `VK_FORMAT_B8G8R8A8_SRGB` — sRGB-encoded 8-bit BGRA (a common sRGB swapchain
    /// format). See [`Self::R8G8B8A8Srgb`].
    B8G8R8A8Srgb = 50,
    /// `VK_FORMAT_R8_SNORM` — a single signed-normalized 8-bit channel mapping the
    /// byte range onto `[-1, 1]` (SDF brick-atlas campaign M0/M1: the quantized
    /// narrow-band distance the 8³ brick atlas stores; the sampler decodes a code to
    /// its `[-band_half, band_half]` distance).
    ///
    /// BUG-M2-GPU-1: this was 9 (`VK_FORMAT_R8_UNORM`), so the atlas image+view were
    /// created as UNORM and the sampler decoded byte 127 as `127/255 = 0.498` instead of
    /// the SNORM `127/127 = 1.0`. The whole M2 cubic was degenerate (corners 0.498× too
    /// small, no sign change → no root → dead branch). The real `VK_FORMAT_R8_SNORM` is 10.
    R8Snorm = 10,
    /// `VK_FORMAT_R8_UNORM` — a single unsigned-normalized 8-bit channel mapping the byte
    /// range onto `[0, 1]` (Render P7: the SSAO term `gSsao` — a full-res `R8_UNORM` STORAGE
    /// image the deferred resolve loads under the `ssao_mode != 0` gate; OFF every pre-P7
    /// scene, so the image is allocated-but-unread). `VK_FORMAT_R8_UNORM` is 9.
    R8Unorm = 9,
    /// `VK_FORMAT_R16_SFLOAT` — a compact single-channel float (deferred SDF use).
    R16Sfloat = 76,
    /// `VK_FORMAT_R16G16_SFLOAT` — two 16-bit half floats (SDFDDGI I1: the probe
    /// DEPTH/visibility atlas storing the two Chebyshev moments `E[d]`/`E[d²]` in the
    /// `.r`/`.g` lanes — the RG16F two-moment depth tile, Decision D2).
    ///
    /// The value is the canonical `VkFormat` enumerant `VK_FORMAT_R16G16_SFLOAT == 83`
    /// (the 16-bit-per-component SFLOAT block: R16=76, R16G16=83). VERIFIED against the
    /// Vulkan spec enumerant, NOT a copied guess (the M2 UNORM-vs-SNORM lesson: a wrong
    /// format const is a silent dead-branch bug — the image + view would be created at
    /// the wrong layout and the sampler decode would be degenerate).
    R16G16Sfloat = 83,
    /// `VK_FORMAT_B10G11R11_UFLOAT_PACK32` — the packed R11G11B10 unsigned-float HDR
    /// format (SDFDDGI I1: the probe IRRADIANCE atlas, Decision D6 — stored WITHOUT the
    /// gamma encode so the resolve path is bit-exact). Despite the "R11G11B10F" shorthand
    /// the ONLY Vulkan format for it packs the components as B10-G11-R11 into one `u32`
    /// (`.r` = low 11 bits, `.g` = next 11, `.b` = high 10); the sampler returns them in
    /// RGB order, so the shader still reads `.rgb` as red/green/blue.
    ///
    /// The value is the canonical `VkFormat` enumerant
    /// `VK_FORMAT_B10G11R11_UFLOAT_PACK32 == 122` (the packed-32 specials block). VERIFIED
    /// against the Vulkan spec enumerant (the M2 lesson: validate the ACTUAL const — there
    /// is NO `VK_FORMAT_R11G11B10_*`, only this B10G11R11 packing).
    B10G11R11UfloatPack32 = 122,
    /// `VK_FORMAT_R32_SFLOAT` — a single 32-bit float (Lighting L0b: the `gViewT`
    /// G-buffer lane storing the marcher's surface ray parameter `t` for world-position
    /// reconstruction in the deferred resolve).
    R32Sfloat = 100,
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
    /// `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL` — read-only sampling by a shader
    /// (Phase-6 S0 rung 5: the layout a sampled texture must be in for a
    /// COMBINED_IMAGE_SAMPLER read in the fragment stage).
    ShaderReadOnlyOptimal = 5,
    /// `VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL` — a transfer/copy source.
    TransferSrcOptimal = 6,
    /// `VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL` — a transfer/copy destination.
    TransferDstOptimal = 7,
    /// `VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL` — bound as a depth attachment
    /// (Vulkan 1.2 core, Phase-6 S0 rung 4). The depth-only counterpart of
    /// [`Self::ColorAttachmentOptimal`]; no stencil aspect.
    DepthAttachmentOptimal = 1_000_241_000,
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
    /// `VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT` (Phase-6 S0 rung 4: the depth
    /// buffer). An image carrying this bit is created with a DEPTH-aspect view.
    pub const DEPTH_STENCIL_ATTACHMENT: ImageUsage = ImageUsage(0x0000_0020);

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

/// A sampler magnification/minification filter (`VkFilter` family, Phase-6 S0
/// rung 5).
///
/// `#[repr(i32)]`; discriminants equal the matching `VkFilter` constants (asserted
/// backend-side). Rung 5 picks [`Self::Nearest`] for a deterministic 1:1 sample
/// (no interpolation across texels), so a sampled texel maps to exactly one source
/// texel and the golden assertion is exact.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    /// `VK_FILTER_NEAREST` — nearest-texel sampling (no interpolation).
    Nearest = 0,
    /// `VK_FILTER_LINEAR` — bilinear interpolation between texels.
    Linear = 1,
}

impl Filter {
    /// The raw `i32` discriminant — equal to the matching `VkFilter` constant.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A sampler texture-coordinate address mode (`VkSamplerAddressMode` family,
/// Phase-6 S0 rung 5).
///
/// `#[repr(i32)]`; discriminants equal the matching `VkSamplerAddressMode`
/// constants (asserted backend-side). Rung 5 uses [`Self::ClampToEdge`] (the
/// simplest deterministic mode — an out-of-`[0, 1]` UV clamps to the edge texel
/// rather than wrapping or hitting a border color).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressMode {
    /// `VK_SAMPLER_ADDRESS_MODE_REPEAT` — wrap the coordinate.
    Repeat = 0,
    /// `VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE` — clamp to the nearest edge texel.
    ClampToEdge = 2,
}

impl AddressMode {
    /// The raw `i32` discriminant — equal to the matching `VkSamplerAddressMode`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// The kind of resource a single bind-group binding holds (the `VkDescriptorType`
/// `i32` family, Render P1a).
///
/// `#[repr(i32)]` with discriminants equal to the matching `VkDescriptorType`
/// constants (asserted backend-side in `abi_guard.rs`), so the backend maps a
/// [`crate::device::BindGroupLayoutEntry::kind`] to a `VkDescriptorType` with a
/// trivial `as i32` cast — no per-kind translation table. Only the five kinds the
/// G-buffer foundation needs are defined (a combined image+sampler, a separate
/// sampled image, a storage image, a uniform buffer, a storage buffer); the family
/// grows per phase. This generalizes the prior COMBINED_IMAGE_SAMPLER-only
/// bind-group surface into the multi-resource descriptor vocabulary.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorKind {
    /// `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER` — a sampled image bundled with
    /// its sampler in one binding (the existing fragment-shader sampling path).
    CombinedImageSampler = 1,
    /// `VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE` — a sampled image with the sampler bound
    /// separately (reserved; the G-buffer marcher samples via combined today).
    SampledImage = 2,
    /// `VK_DESCRIPTOR_TYPE_STORAGE_IMAGE` — a read/write image a compute shader
    /// stores into (the P1a marcher's output target).
    StorageImage = 3,
    /// `VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER` — a read-only constant buffer (reserved
    /// for the G-buffer's per-frame uniforms).
    UniformBuffer = 6,
    /// `VK_DESCRIPTOR_TYPE_STORAGE_BUFFER` — a read/write buffer a shader accesses
    /// (the P1a marcher's edit-list input).
    StorageBuffer = 7,
}

impl DescriptorKind {
    /// The raw `i32` discriminant — equal to the matching `VkDescriptorType`.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A depth/comparison operation (the `VkCompareOp` `i32` family, CSM Increment 0).
///
/// `#[repr(i32)]` with discriminants equal to the matching `VkCompareOp` constants
/// (asserted backend-side in `abi_guard.rs`), so the backend lowers it with a
/// trivial `as i32` cast — no per-op translation table. Surfaced for the comparison
/// sampler ([`crate::device::SamplerDesc::compare`]): a shadow-map PCF read needs
/// [`Self::LessOrEqual`] (a depth-array texel passes the hardware compare iff the
/// reference depth is `<=` the stored depth). Only the ops the foundation needs are
/// defined; the family grows per phase.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    /// `VK_COMPARE_OP_NEVER` — the test never passes.
    Never = 0,
    /// `VK_COMPARE_OP_LESS` — passes iff `reference < stored`.
    Less = 1,
    /// `VK_COMPARE_OP_LESS_OR_EQUAL` — passes iff `reference <= stored` (PCF shadow
    /// comparison: a fragment at the stored depth is lit, not self-shadowed).
    LessOrEqual = 3,
    /// `VK_COMPARE_OP_ALWAYS` — the test always passes.
    Always = 7,
}

impl CompareOp {
    /// The raw `i32` discriminant — equal to the matching `VkCompareOp` constant.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A triangle face-culling mode (the `VkCullModeFlags` `u32` family, CSM
/// Increment 0).
///
/// `#[repr(u32)]` with discriminants equal to the matching `VK_CULL_MODE_*`
/// constants (asserted backend-side in `abi_guard.rs`), so the backend lowers it
/// with a trivial `as u32` (= `VkFlags`) cast. Surfaced on
/// [`crate::descriptor::GraphicsPipelineDesc::cull_mode`]: [`Self::None`] is the
/// engine default (every existing pipeline — byte-identical to today's hardcoded
/// `VK_CULL_MODE_NONE`); a shadow-map depth pass selects [`Self::Front`] to reduce
/// peter-panning by rendering back faces. The variant set covers the single-bit
/// modes; `FRONT_AND_BACK` is intentionally omitted (it discards all geometry).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CullMode {
    /// `VK_CULL_MODE_NONE` — no culling (the engine default; today's hardcoded value).
    None = 0,
    /// `VK_CULL_MODE_FRONT_BIT` — cull front-facing triangles.
    Front = 1,
    /// `VK_CULL_MODE_BACK_BIT` — cull back-facing triangles.
    Back = 2,
}

impl CullMode {
    /// The raw `u32` discriminant — equal to the matching `VkCullModeFlags` bits
    /// (`VkFlags`).
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

/// A color-blend factor (the `VkBlendFactor` `i32` family, GUI P5a Decision 3).
///
/// `#[repr(i32)]` with discriminants equal to the matching `VkBlendFactor`
/// constants (asserted backend-side in `abi_guard.rs`), so the backend lowers a
/// [`BlendState`] factor with a trivial `as i32` cast — no per-factor translation
/// table. Only the factors the premultiplied/straight-alpha UI blend needs are
/// defined; the family grows per phase.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendFactor {
    /// `VK_BLEND_FACTOR_ZERO`.
    Zero = 0,
    /// `VK_BLEND_FACTOR_ONE`.
    One = 1,
    /// `VK_BLEND_FACTOR_SRC_ALPHA`.
    SrcAlpha = 6,
    /// `VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA`.
    OneMinusSrcAlpha = 7,
}

impl BlendFactor {
    /// The raw `i32` discriminant — equal to the matching `VkBlendFactor` constant.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A color-blend operation (the `VkBlendOp` `i32` family, GUI P5a Decision 3).
///
/// `#[repr(i32)]`; discriminants equal the matching `VkBlendOp` constants (asserted
/// backend-side). Only [`Self::Add`] (the only op premultiplied/straight alpha
/// needs) is defined; the family grows per phase.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendOp {
    /// `VK_BLEND_OP_ADD` — `src * srcFactor (op) dst * dstFactor`.
    Add = 0,
}

impl BlendOp {
    /// The raw `i32` discriminant — equal to the matching `VkBlendOp` constant.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A single color attachment's blend state (GUI P5a Decision 3).
///
/// `#[repr(C)]` POD with an explicit field order so a backend reads it without
/// depending on Rust's default field reordering — it lowers onto a
/// `VkPipelineColorBlendAttachmentState`'s `(srcColor, dstColor, colorOp, srcAlpha,
/// dstAlpha, alphaOp)`. Carried as `Option<BlendState>` on
/// [`crate::descriptor::GraphicsPipelineDesc`]: `None` keeps the engine's default
/// opaque (blend-disabled) write for every existing pipeline; `Some(bs)` enables
/// blending with these factors/ops on the (single) color attachment.
///
/// The UI pipeline passes [`Self::PREMULTIPLIED_ALPHA`] (RmlUi/WebRender default):
/// AA edges over a transparent backdrop compose correctly under nested clips and
/// future world-space layering, where straight alpha would fringe/double-darken.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlendState {
    /// The factor the source color is multiplied by.
    pub src_color: BlendFactor,
    /// The factor the destination color is multiplied by.
    pub dst_color: BlendFactor,
    /// How the weighted source + destination colors combine.
    pub color_op: BlendOp,
    /// The factor the source alpha is multiplied by.
    pub src_alpha: BlendFactor,
    /// The factor the destination alpha is multiplied by.
    pub dst_alpha: BlendFactor,
    /// How the weighted source + destination alphas combine.
    pub alpha_op: BlendOp,
}

impl BlendState {
    /// Premultiplied-alpha over: `src + dst * (1 - src_a)` for both color and alpha
    /// (`srcColor=ONE, dstColor=ONE_MINUS_SRC_ALPHA, srcAlpha=ONE,
    /// dstAlpha=ONE_MINUS_SRC_ALPHA, op=ADD`). The engine UI default — the fragment
    /// shader emits `(rgb*a*cov, a*cov)` so coverage-multiplied output composes
    /// correctly. Requires the source color to already be premultiplied.
    pub const PREMULTIPLIED_ALPHA: BlendState = BlendState {
        src_color: BlendFactor::One,
        dst_color: BlendFactor::OneMinusSrcAlpha,
        color_op: BlendOp::Add,
        src_alpha: BlendFactor::One,
        dst_alpha: BlendFactor::OneMinusSrcAlpha,
        alpha_op: BlendOp::Add,
    };

    /// Straight (non-premultiplied) alpha over: `src*src_a + dst*(1 - src_a)`
    /// color, `src_a + dst*(1 - src_a)` alpha. Defined for callers that author
    /// straight-alpha source colors; the UI path uses
    /// [`Self::PREMULTIPLIED_ALPHA`].
    pub const STRAIGHT_ALPHA: BlendState = BlendState {
        src_color: BlendFactor::SrcAlpha,
        dst_color: BlendFactor::OneMinusSrcAlpha,
        color_op: BlendOp::Add,
        src_alpha: BlendFactor::One,
        dst_alpha: BlendFactor::OneMinusSrcAlpha,
        alpha_op: BlendOp::Add,
    };
}

/// Image-aspect bitflags for an image subresource. Values equal the
/// `VK_IMAGE_ASPECT_*` bits.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageAspect(u32);

impl ImageAspect {
    /// `VK_IMAGE_ASPECT_COLOR_BIT`.
    pub const COLOR: ImageAspect = ImageAspect(0x0000_0001);
    /// `VK_IMAGE_ASPECT_DEPTH_BIT` (Phase-6 S0 rung 4: the depth attachment +
    /// its barrier/subresource range).
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

/// The pipeline stage a [`crate::encoder::RhiCommandEncoder::write_timestamp`]
/// captures (HW-RT rung R0: the profiler-standard TOP/BOTTOM bracket).
///
/// A SINGLE `VkPipelineStageFlagBits` (NOT the [`BarrierStage`] bitmask): a
/// `vkCmdWriteTimestamp` takes one pipeline-stage bit naming the moment the query
/// is written. `#[repr(i32)]` with discriminants equal to the matching
/// `VK_PIPELINE_STAGE_*` bit values, so the Vulkan backend lowers it with a trivial
/// `as VkFlags` cast (asserted backend-side). A bracket opens at [`Self::TopOfPipe`]
/// (front of the passed pass) and closes at [`Self::BottomOfPipe`] (its retirement);
/// intermediate stages are meaningless for the pass-wall-clock measurement.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimestampStage {
    /// `VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT` — the front of the pipeline (the bracket
    /// open, capturing the moment work enters the pass).
    TopOfPipe = 0x0000_0001,
    /// `VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT` — the back of the pipeline (the bracket
    /// close, capturing the moment the pass retires).
    BottomOfPipe = 0x0000_2000,
}

impl TimestampStage {
    /// The raw `i32` discriminant — equal to the matching `VK_PIPELINE_STAGE_*` bit
    /// (a `VkFlags`/`u32` value that fits an `i32` for these two low bits).
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}
