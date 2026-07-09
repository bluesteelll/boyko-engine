//! Compile-time enforcement of the identity-cast contract (plan B1 / ABI-3 /
//! SEAM-3).
//!
//! The backend translates `boyko_rhi`'s agnostic bitflag enums to Vulkan flags by
//! a **no-op identity cast** (`desc.usage.bits()` fed straight into a `VkFlags`),
//! relying on the agnostic bit values being numerically EQUAL to the
//! corresponding `VK_*` constants (plan D5). `boyko_rhi` has no FFI dependency, so
//! it cannot assert this equality itself. This module asserts it **here**, in the
//! backend that owns both sides, with `const _: () = assert!(...)` blocks: any
//! future drift between an agnostic bit and its `VK_*` constant breaks the build
//! instead of silently corrupting a buffer-usage / stage / access mask.
//!
//! Every bit the backend identity-casts (`rhi_impl/device.rs::create_buffer`,
//! `push_constants`, `pipeline_barrier`) is covered.

use boyko_rhi::enums::{
    AddressMode, BarrierAccess, BarrierStage, BlendFactor, BlendOp, BufferUsage, CompareOp,
    CullMode, DescriptorKind, Filter, Format, ImageAspect, ImageLayout, ImageUsage, IndexType,
    LoadOp, PrimitiveTopology, ShaderStage, StoreOp, TextureDimension, TimestampStage, VertexFormat,
};

use crate::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT,
    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT,
    VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
    VK_ATTACHMENT_LOAD_OP_CLEAR, VK_ATTACHMENT_LOAD_OP_LOAD, VK_ATTACHMENT_STORE_OP_STORE,
    VK_BLEND_FACTOR_ONE, VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA, VK_BLEND_FACTOR_SRC_ALPHA,
    VK_BLEND_FACTOR_ZERO, VK_BLEND_OP_ADD, VK_BUFFER_USAGE_INDEX_BUFFER_BIT,
    VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
    VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, VK_BUFFER_USAGE_TRANSFER_DST_BIT,
    VK_BUFFER_USAGE_TRANSFER_SRC_BIT, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT,
    VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, VK_COMPARE_OP_ALWAYS, VK_COMPARE_OP_LESS,
    VK_COMPARE_OP_LESS_OR_EQUAL, VK_COMPARE_OP_NEVER, VK_CULL_MODE_BACK_BIT, VK_CULL_MODE_FRONT_BIT,
    VK_CULL_MODE_NONE, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
    VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
    VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, VK_FILTER_LINEAR,
    VK_FILTER_NEAREST, VK_FORMAT_B10G11R11_UFLOAT_PACK32, VK_FORMAT_B8G8R8A8_SRGB,
    VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_D32_SFLOAT, VK_FORMAT_R16G16B16A16_UNORM,
    VK_FORMAT_R16G16_SFLOAT, VK_FORMAT_R16G16_UNORM,
    VK_FORMAT_R16_SFLOAT, VK_FORMAT_R32G32B32A32_SFLOAT, VK_FORMAT_R32G32B32_SFLOAT,
    VK_FORMAT_R32_SFLOAT, VK_FORMAT_R8G8B8A8_SRGB, VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_R8G8_UNORM,
    VK_FORMAT_R8_SNORM, VK_FORMAT_R8_UNORM,
    VK_FORMAT_UNDEFINED,
    VK_IMAGE_ASPECT_COLOR_BIT,
    VK_IMAGE_ASPECT_DEPTH_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_GENERAL,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_TYPE_2D,
    VK_IMAGE_TYPE_3D, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
    VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT, VK_IMAGE_USAGE_SAMPLED_BIT,
    VK_IMAGE_USAGE_STORAGE_BIT, VK_IMAGE_USAGE_TRANSFER_DST_BIT, VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
    VK_INDEX_TYPE_UINT16, VK_INDEX_TYPE_UINT32, VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
    VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
    VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
    VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE, VK_SAMPLER_ADDRESS_MODE_REPEAT,
    VK_SHADER_STAGE_COMPUTE_BIT, VK_SHADER_STAGE_FRAGMENT_BIT, VK_SHADER_STAGE_VERTEX_BIT,
};

// ===== BufferUsage (identity-cast in `create_buffer`). =====
const _: () = assert!(
    BufferUsage::TRANSFER_SRC.bits() == VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
    "BufferUsage::TRANSFER_SRC must equal VK_BUFFER_USAGE_TRANSFER_SRC_BIT"
);
const _: () = assert!(
    BufferUsage::TRANSFER_DST.bits() == VK_BUFFER_USAGE_TRANSFER_DST_BIT,
    "BufferUsage::TRANSFER_DST must equal VK_BUFFER_USAGE_TRANSFER_DST_BIT"
);
const _: () = assert!(
    BufferUsage::UNIFORM.bits() == VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT,
    "BufferUsage::UNIFORM must equal VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT"
);
const _: () = assert!(
    BufferUsage::STORAGE.bits() == VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
    "BufferUsage::STORAGE must equal VK_BUFFER_USAGE_STORAGE_BUFFER_BIT"
);
const _: () = assert!(
    BufferUsage::INDIRECT.bits() == VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
    "BufferUsage::INDIRECT must equal VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT"
);
const _: () = assert!(
    BufferUsage::VERTEX.bits() == VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
    "BufferUsage::VERTEX must equal VK_BUFFER_USAGE_VERTEX_BUFFER_BIT"
);
const _: () = assert!(
    BufferUsage::INDEX.bits() == VK_BUFFER_USAGE_INDEX_BUFFER_BIT,
    "BufferUsage::INDEX must equal VK_BUFFER_USAGE_INDEX_BUFFER_BIT"
);

// ===== HW-RT rung R2a-2 — AS buffer-usage bits (identity-cast in `create_buffer`;
// gated `hwrt` because they reference the hwrt-only VK usage consts). =====
#[cfg(feature = "hwrt")]
const _: () = assert!(
    boyko_rhi::BufferUsage::SHADER_DEVICE_ADDRESS.bits()
        == crate::ffi::VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT,
    "BufferUsage::SHADER_DEVICE_ADDRESS must equal VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT"
);
#[cfg(feature = "hwrt")]
const _: () = assert!(
    boyko_rhi::BufferUsage::ACCEL_STRUCTURE_STORAGE.bits()
        == crate::ffi::VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR,
    "BufferUsage::ACCEL_STRUCTURE_STORAGE must equal VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR"
);
#[cfg(feature = "hwrt")]
const _: () = assert!(
    boyko_rhi::BufferUsage::ACCEL_BUILD_INPUT.bits()
        == crate::ffi::VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR,
    "BufferUsage::ACCEL_BUILD_INPUT must equal VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR"
);

// ===== ShaderStage (identity-cast in `push_constants`). =====
const _: () = assert!(
    ShaderStage::VERTEX.bits() == VK_SHADER_STAGE_VERTEX_BIT,
    "ShaderStage::VERTEX must equal VK_SHADER_STAGE_VERTEX_BIT"
);
const _: () = assert!(
    ShaderStage::FRAGMENT.bits() == VK_SHADER_STAGE_FRAGMENT_BIT,
    "ShaderStage::FRAGMENT must equal VK_SHADER_STAGE_FRAGMENT_BIT"
);
const _: () = assert!(
    ShaderStage::COMPUTE.bits() == VK_SHADER_STAGE_COMPUTE_BIT,
    "ShaderStage::COMPUTE must equal VK_SHADER_STAGE_COMPUTE_BIT"
);

// ===== BarrierStage (identity-cast in `pipeline_barrier`). =====
const _: () = assert!(
    BarrierStage::COMPUTE_SHADER.bits() == VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    "BarrierStage::COMPUTE_SHADER must equal VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT"
);
const _: () = assert!(
    BarrierStage::TRANSFER.bits() == VK_PIPELINE_STAGE_TRANSFER_BIT,
    "BarrierStage::TRANSFER must equal VK_PIPELINE_STAGE_TRANSFER_BIT"
);

// ===== BarrierAccess (identity-cast in `pipeline_barrier`). =====
const _: () = assert!(
    BarrierAccess::SHADER_READ.bits() == VK_ACCESS_SHADER_READ_BIT,
    "BarrierAccess::SHADER_READ must equal VK_ACCESS_SHADER_READ_BIT"
);
const _: () = assert!(
    BarrierAccess::SHADER_WRITE.bits() == VK_ACCESS_SHADER_WRITE_BIT,
    "BarrierAccess::SHADER_WRITE must equal VK_ACCESS_SHADER_WRITE_BIT"
);
const _: () = assert!(
    BarrierAccess::TRANSFER_READ.bits() == VK_ACCESS_TRANSFER_READ_BIT,
    "BarrierAccess::TRANSFER_READ must equal VK_ACCESS_TRANSFER_READ_BIT"
);
const _: () = assert!(
    BarrierAccess::TRANSFER_WRITE.bits() == VK_ACCESS_TRANSFER_WRITE_BIT,
    "BarrierAccess::TRANSFER_WRITE must equal VK_ACCESS_TRANSFER_WRITE_BIT"
);

// ===========================================================================
// Phase-6 S0 graphics-surface contracts. The new `image_barrier` adds graphics
// stage/access bits (identity-cast in `image_barrier`); `create_texture` +
// `begin_rendering` + `copy_image_to_buffer` map the `Format`/`ImageLayout`/
// `ImageUsage`/`TextureDimension`/`LoadOp`/`StoreOp`/`ImageAspect` families. The
// `i32` families (`Format`/`ImageLayout`/…) are NOT identity-cast bitflags (plan
// D5/W1) but their `as_i32()` discriminants still equal the `VK_*` constants, so
// the backend's `as_i32()` lowering is a no-op — these asserts pin that equality.
// ===========================================================================

// --- BarrierStage graphics bits (identity-cast in `image_barrier`). ---
const _: () = assert!(
    BarrierStage::TOP_OF_PIPE.bits() == VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
    "BarrierStage::TOP_OF_PIPE must equal VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT"
);
const _: () = assert!(
    BarrierStage::COLOR_ATTACHMENT_OUTPUT.bits() == VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    "BarrierStage::COLOR_ATTACHMENT_OUTPUT must equal VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT"
);
const _: () = assert!(
    BarrierStage::BOTTOM_OF_PIPE.bits() == VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
    "BarrierStage::BOTTOM_OF_PIPE must equal VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT"
);

// --- BarrierAccess graphics bit (identity-cast in `image_barrier`). ---
const _: () = assert!(
    BarrierAccess::COLOR_ATTACHMENT_WRITE.bits() == VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
    "BarrierAccess::COLOR_ATTACHMENT_WRITE must equal VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT"
);

// --- TimestampStage `as_i32()` (identity-cast in `write_timestamp`, HW-RT rung R0). ---
const _: () = assert!(
    TimestampStage::TopOfPipe.as_i32() == VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT as i32,
    "TimestampStage::TopOfPipe must equal VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT"
);
const _: () = assert!(
    TimestampStage::BottomOfPipe.as_i32() == VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT as i32,
    "TimestampStage::BottomOfPipe must equal VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT"
);

// --- ImageUsage (identity-cast in `create_texture`). ---
const _: () = assert!(
    ImageUsage::TRANSFER_SRC.bits() == VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
    "ImageUsage::TRANSFER_SRC must equal VK_IMAGE_USAGE_TRANSFER_SRC_BIT"
);
const _: () = assert!(
    ImageUsage::TRANSFER_DST.bits() == VK_IMAGE_USAGE_TRANSFER_DST_BIT,
    "ImageUsage::TRANSFER_DST must equal VK_IMAGE_USAGE_TRANSFER_DST_BIT"
);
const _: () = assert!(
    ImageUsage::SAMPLED.bits() == VK_IMAGE_USAGE_SAMPLED_BIT,
    "ImageUsage::SAMPLED must equal VK_IMAGE_USAGE_SAMPLED_BIT"
);
const _: () = assert!(
    ImageUsage::STORAGE.bits() == VK_IMAGE_USAGE_STORAGE_BIT,
    "ImageUsage::STORAGE must equal VK_IMAGE_USAGE_STORAGE_BIT"
);
const _: () = assert!(
    ImageUsage::COLOR_ATTACHMENT.bits() == VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
    "ImageUsage::COLOR_ATTACHMENT must equal VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT"
);

// --- ImageAspect (identity-cast in `image_barrier` / `copy_image_to_buffer`). ---
const _: () = assert!(
    ImageAspect::COLOR.bits() == VK_IMAGE_ASPECT_COLOR_BIT,
    "ImageAspect::COLOR must equal VK_IMAGE_ASPECT_COLOR_BIT"
);

// --- Format `as_i32()` (mapped in `create_texture`; rung-1 subset). ---
const _: () = assert!(
    Format::Undefined.as_i32() == VK_FORMAT_UNDEFINED,
    "Format::Undefined must equal VK_FORMAT_UNDEFINED"
);
const _: () = assert!(
    Format::R8G8B8A8Unorm.as_i32() == VK_FORMAT_R8G8B8A8_UNORM,
    "Format::R8G8B8A8Unorm must equal VK_FORMAT_R8G8B8A8_UNORM"
);
const _: () = assert!(
    Format::B8G8R8A8Unorm.as_i32() == VK_FORMAT_B8G8R8A8_UNORM,
    "Format::B8G8R8A8Unorm must equal VK_FORMAT_B8G8R8A8_UNORM"
);
const _: () = assert!(
    Format::R8G8B8A8Srgb.as_i32() == VK_FORMAT_R8G8B8A8_SRGB,
    "Format::R8G8B8A8Srgb must equal VK_FORMAT_R8G8B8A8_SRGB"
);
const _: () = assert!(
    Format::B8G8R8A8Srgb.as_i32() == VK_FORMAT_B8G8R8A8_SRGB,
    "Format::B8G8R8A8Srgb must equal VK_FORMAT_B8G8R8A8_SRGB"
);
// Lighting L0b: the `gViewT` G-buffer lane format (`as_i32()` mapped in `create_texture`).
const _: () = assert!(
    Format::R32Sfloat.as_i32() == VK_FORMAT_R32_SFLOAT,
    "Format::R32Sfloat must equal VK_FORMAT_R32_SFLOAT"
);
// SDF brick-atlas campaign: the quantized narrow-band brick format (`as_i32()`
// mapped in `create_texture`; image-binding wired in M1).
const _: () = assert!(
    Format::R8Snorm.as_i32() == VK_FORMAT_R8_SNORM,
    "Format::R8Snorm must equal VK_FORMAT_R8_SNORM"
);
// Render P7: the SSAO term `gSsao` format (`as_i32()` mapped in `create_texture`).
const _: () = assert!(
    Format::R8Unorm.as_i32() == VK_FORMAT_R8_UNORM,
    "Format::R8Unorm must equal VK_FORMAT_R8_UNORM"
);
// SDF brick-atlas campaign M2: the D8 atlas fallback format (`as_i32()` mapped in
// `create_texture`; chosen by the `atlas_linear_filter_ok` probe when `R8_SNORM` lacks
// the linear-filter feature).
const _: () = assert!(
    Format::R16Sfloat.as_i32() == VK_FORMAT_R16_SFLOAT,
    "Format::R16Sfloat must equal VK_FORMAT_R16_SFLOAT"
);
// SDFDDGI I1: the probe DEPTH/visibility two-moment atlas format (`as_i32()` mapped in
// `create_texture`). The M2 lesson net: pin against the canonical enumerant, not a copy.
const _: () = assert!(
    Format::R16G16Sfloat.as_i32() == VK_FORMAT_R16G16_SFLOAT,
    "Format::R16G16Sfloat must equal VK_FORMAT_R16G16_SFLOAT"
);
// Rung 3a: the RT soft-shadow VISIBILITY target `shadow_vis` format (`as_i32()` mapped in
// `create_texture`). The M2 lesson net: pin against the canonical enumerant, not a copy.
const _: () = assert!(
    Format::R8G8Unorm.as_i32() == VK_FORMAT_R8G8_UNORM,
    "Format::R8G8Unorm must equal VK_FORMAT_R8G8_UNORM"
);
// Rung 3a: the à-trous ping-pong target `shadow_vis2` format (`as_i32()` mapped in
// `create_texture`). Pinned to the canonical enumerant (`VK_FORMAT_R16G16_UNORM == 77`).
// HW-RT Rung 3b: the temporal shadow-vis HISTORY ring format (`as_i32()` mapped in
// `create_texture`). Pinned to the canonical enumerant (`VK_FORMAT_R16G16B16A16_UNORM == 91`).
const _: () = assert!(
    Format::R16G16B16A16Unorm.as_i32() == VK_FORMAT_R16G16B16A16_UNORM,
    "Format::R16G16B16A16Unorm must equal VK_FORMAT_R16G16B16A16_UNORM"
);
const _: () = assert!(
    Format::R16G16Unorm.as_i32() == VK_FORMAT_R16G16_UNORM,
    "Format::R16G16Unorm must equal VK_FORMAT_R16G16_UNORM"
);
// SDFDDGI I1: the probe IRRADIANCE atlas format (`R11G11B10F`-no-gamma, Decision D6;
// `as_i32()` mapped in `create_texture`).
const _: () = assert!(
    Format::B10G11R11UfloatPack32.as_i32() == VK_FORMAT_B10G11R11_UFLOAT_PACK32,
    "Format::B10G11R11UfloatPack32 must equal VK_FORMAT_B10G11R11_UFLOAT_PACK32"
);

// --- ImageLayout `as_i32()` (mapped in `image_barrier` / `begin_rendering`). ---
const _: () = assert!(
    ImageLayout::Undefined.as_i32() == VK_IMAGE_LAYOUT_UNDEFINED,
    "ImageLayout::Undefined must equal VK_IMAGE_LAYOUT_UNDEFINED"
);
const _: () = assert!(
    ImageLayout::General.as_i32() == VK_IMAGE_LAYOUT_GENERAL,
    "ImageLayout::General must equal VK_IMAGE_LAYOUT_GENERAL"
);
const _: () = assert!(
    ImageLayout::ColorAttachmentOptimal.as_i32() == VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
    "ImageLayout::ColorAttachmentOptimal must equal VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL"
);
const _: () = assert!(
    ImageLayout::TransferSrcOptimal.as_i32() == VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    "ImageLayout::TransferSrcOptimal must equal VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL"
);
const _: () = assert!(
    ImageLayout::TransferDstOptimal.as_i32() == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
    "ImageLayout::TransferDstOptimal must equal VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL"
);

// --- TextureDimension `as_i32()` (mapped in `create_texture`). ---
const _: () = assert!(
    TextureDimension::D2.as_i32() == VK_IMAGE_TYPE_2D,
    "TextureDimension::D2 must equal VK_IMAGE_TYPE_2D"
);
const _: () = assert!(
    TextureDimension::D3.as_i32() == VK_IMAGE_TYPE_3D,
    "TextureDimension::D3 must equal VK_IMAGE_TYPE_3D"
);

// --- LoadOp / StoreOp `as_i32()` (mapped in `begin_rendering`). ---
const _: () = assert!(
    LoadOp::Load.as_i32() == VK_ATTACHMENT_LOAD_OP_LOAD,
    "LoadOp::Load must equal VK_ATTACHMENT_LOAD_OP_LOAD"
);
const _: () = assert!(
    LoadOp::Clear.as_i32() == VK_ATTACHMENT_LOAD_OP_CLEAR,
    "LoadOp::Clear must equal VK_ATTACHMENT_LOAD_OP_CLEAR"
);
const _: () = assert!(
    StoreOp::Store.as_i32() == VK_ATTACHMENT_STORE_OP_STORE,
    "StoreOp::Store must equal VK_ATTACHMENT_STORE_OP_STORE"
);

// ===========================================================================
// Phase-6 S0 rung-2 graphics-pipeline contracts. `PrimitiveTopology` `as_i32()` is
// mapped in `create_graphics_pipeline` (the `VkPipelineInputAssemblyStateCreateInfo`
// topology); the `Viewport`↔`VkViewport` byte-for-byte layout match (a direct slice
// reinterpret in `set_viewport`) is asserted at the cast site in `rhi_impl/encoder.rs`.
// ===========================================================================

// --- PrimitiveTopology `as_i32()` (mapped in `create_graphics_pipeline`). ---
const _: () = assert!(
    PrimitiveTopology::TriangleList.as_i32() == VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
    "PrimitiveTopology::TriangleList must equal VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST"
);

// --- BlendFactor / BlendOp `as_i32()` (mapped in `create_graphics_pipeline`'s
//     color-blend lowering, GUI P5a Decision 3). ---
const _: () = assert!(
    BlendFactor::Zero.as_i32() == VK_BLEND_FACTOR_ZERO,
    "BlendFactor::Zero must equal VK_BLEND_FACTOR_ZERO"
);
const _: () = assert!(
    BlendFactor::One.as_i32() == VK_BLEND_FACTOR_ONE,
    "BlendFactor::One must equal VK_BLEND_FACTOR_ONE"
);
const _: () = assert!(
    BlendFactor::SrcAlpha.as_i32() == VK_BLEND_FACTOR_SRC_ALPHA,
    "BlendFactor::SrcAlpha must equal VK_BLEND_FACTOR_SRC_ALPHA"
);
const _: () = assert!(
    BlendFactor::OneMinusSrcAlpha.as_i32() == VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
    "BlendFactor::OneMinusSrcAlpha must equal VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA"
);
const _: () = assert!(
    BlendOp::Add.as_i32() == VK_BLEND_OP_ADD,
    "BlendOp::Add must equal VK_BLEND_OP_ADD"
);

// ===========================================================================
// Phase-6 S0 rung-3 vertex-input + index contracts. `VertexFormat` `as_i32()` is
// mapped in `create_graphics_pipeline` (the `VkVertexInputAttributeDescription`
// format); `IndexType` `as_i32()` is mapped in `bind_index_buffer`. These pin the
// agnostic↔`VkFormat`/`VkIndexType` discriminant equality.
// ===========================================================================

// --- VertexFormat `as_i32()` (mapped in `create_graphics_pipeline`). ---
const _: () = assert!(
    VertexFormat::Float32x3.as_i32() == VK_FORMAT_R32G32B32_SFLOAT,
    "VertexFormat::Float32x3 must equal VK_FORMAT_R32G32B32_SFLOAT"
);
const _: () = assert!(
    VertexFormat::Float32x4.as_i32() == VK_FORMAT_R32G32B32A32_SFLOAT,
    "VertexFormat::Float32x4 must equal VK_FORMAT_R32G32B32A32_SFLOAT"
);

// --- IndexType `as_i32()` (mapped in `bind_index_buffer`; the indexed-draw
//     `draw_indexed`/`vkCmdDrawIndexed` consumes the index buffer bound under this
//     discriminant — its remaining params are plain `u32`/`i32` with no agnostic
//     mapping, so the index-type equality is the whole ABI surface to pin). ---
const _: () = assert!(
    IndexType::Uint16.as_i32() == VK_INDEX_TYPE_UINT16,
    "IndexType::Uint16 must equal VK_INDEX_TYPE_UINT16"
);
const _: () = assert!(
    IndexType::Uint32.as_i32() == VK_INDEX_TYPE_UINT32,
    "IndexType::Uint32 must equal VK_INDEX_TYPE_UINT32"
);

// ===========================================================================
// Phase-6 S0 rung-4 depth contracts. The depth attachment + depth-test pipeline
// state add: the `EARLY/LATE_FRAGMENT_TESTS` stage bits + the
// `DEPTH_STENCIL_ATTACHMENT_READ/WRITE` access bits (identity-cast in
// `image_barrier`); the `Format::D32Sfloat` depth format + the
// `DepthAttachmentOptimal` layout + the `DEPTH_STENCIL_ATTACHMENT` usage + the
// `DEPTH` aspect (mapped in `create_texture` / `create_graphics_pipeline` /
// `begin_rendering` / `image_barrier`). These pin the agnostic↔`VK_*` equality
// the rung-4 lowerings identity-cast / `as_i32()`-lower.
// ===========================================================================

// --- BarrierStage depth-test bits (identity-cast in `image_barrier`). ---
const _: () = assert!(
    BarrierStage::EARLY_FRAGMENT_TESTS.bits() == VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT,
    "BarrierStage::EARLY_FRAGMENT_TESTS must equal VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT"
);
const _: () = assert!(
    BarrierStage::LATE_FRAGMENT_TESTS.bits() == VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
    "BarrierStage::LATE_FRAGMENT_TESTS must equal VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT"
);

// --- BarrierAccess depth-attachment bits (identity-cast in `image_barrier`). ---
const _: () = assert!(
    BarrierAccess::DEPTH_STENCIL_ATTACHMENT_READ.bits()
        == VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT,
    "BarrierAccess::DEPTH_STENCIL_ATTACHMENT_READ must equal VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT"
);
const _: () = assert!(
    BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE.bits()
        == VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    "BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE must equal VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT"
);

// --- ImageUsage depth bit (identity-cast in `create_texture`). ---
const _: () = assert!(
    ImageUsage::DEPTH_STENCIL_ATTACHMENT.bits() == VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT,
    "ImageUsage::DEPTH_STENCIL_ATTACHMENT must equal VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT"
);

// --- ImageAspect depth bit (identity-cast in `image_barrier` / the depth view). ---
const _: () = assert!(
    ImageAspect::DEPTH.bits() == VK_IMAGE_ASPECT_DEPTH_BIT,
    "ImageAspect::DEPTH must equal VK_IMAGE_ASPECT_DEPTH_BIT"
);

// --- Format::D32Sfloat `as_i32()` (mapped in `create_texture` / pipeline). ---
const _: () = assert!(
    Format::D32Sfloat.as_i32() == VK_FORMAT_D32_SFLOAT,
    "Format::D32Sfloat must equal VK_FORMAT_D32_SFLOAT"
);

// --- ImageLayout::DepthAttachmentOptimal `as_i32()` (mapped in `image_barrier` /
//     `begin_rendering`). ---
const _: () = assert!(
    ImageLayout::DepthAttachmentOptimal.as_i32() == VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
    "ImageLayout::DepthAttachmentOptimal must equal VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL"
);

// ===========================================================================
// Phase-6 S0 rung-5 sampler + combined-image-sampler contracts. The sampling
// surface adds: the `FRAGMENT_SHADER` stage bit (identity-cast in `image_barrier`
// for the COLOR → SHADER_READ barrier); the `ShaderReadOnlyOptimal` layout (mapped
// in `image_barrier` / written into the descriptor's image-info); and the
// `Filter`/`AddressMode` `as_i32()` families (mapped in `create_sampler`). These
// pin the agnostic↔`VK_*` equality the rung-5 lowerings identity-cast / `as_i32()`-
// lower. (`ShaderStage::FRAGMENT` for the bind-group-layout stage is already pinned
// above; `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER` is a backend-only constant the
// agnostic surface never names, so it has no agnostic counterpart to assert.)
// ===========================================================================

// --- BarrierStage::FRAGMENT_SHADER (identity-cast in `image_barrier`). ---
const _: () = assert!(
    BarrierStage::FRAGMENT_SHADER.bits() == VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
    "BarrierStage::FRAGMENT_SHADER must equal VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT"
);

// --- ImageLayout::ShaderReadOnlyOptimal `as_i32()` (mapped in `image_barrier` /
//     the bind-group's descriptor image-info). ---
const _: () = assert!(
    ImageLayout::ShaderReadOnlyOptimal.as_i32() == VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
    "ImageLayout::ShaderReadOnlyOptimal must equal VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL"
);

// --- Filter `as_i32()` (mapped in `create_sampler`). ---
const _: () = assert!(
    Filter::Nearest.as_i32() == VK_FILTER_NEAREST,
    "Filter::Nearest must equal VK_FILTER_NEAREST"
);
const _: () = assert!(
    Filter::Linear.as_i32() == VK_FILTER_LINEAR,
    "Filter::Linear must equal VK_FILTER_LINEAR"
);

// --- AddressMode `as_i32()` (mapped in `create_sampler`). ---
const _: () = assert!(
    AddressMode::Repeat.as_i32() == VK_SAMPLER_ADDRESS_MODE_REPEAT,
    "AddressMode::Repeat must equal VK_SAMPLER_ADDRESS_MODE_REPEAT"
);
const _: () = assert!(
    AddressMode::ClampToEdge.as_i32() == VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
    "AddressMode::ClampToEdge must equal VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE"
);

// ===========================================================================
// Render P1a descriptor-vocabulary contract. `DescriptorKind` `as_i32()` is mapped
// in `create_bind_group_layout` / `create_bind_group` (the per-entry
// `VkDescriptorType` of each binding + each `VkWriteDescriptorSet`) by a trivial
// `as i32` cast — so its discriminants MUST equal the matching `VK_DESCRIPTOR_TYPE_*`
// constants. These pin that equality; any drift breaks the build instead of writing
// a descriptor of the wrong type.
// ===========================================================================

// --- DescriptorKind `as_i32()` (mapped in `create_bind_group_layout`/`create_bind_group`). ---
const _: () = assert!(
    DescriptorKind::CombinedImageSampler.as_i32() == VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
    "DescriptorKind::CombinedImageSampler must equal VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER"
);
const _: () = assert!(
    DescriptorKind::SampledImage.as_i32() == VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE,
    "DescriptorKind::SampledImage must equal VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE"
);
const _: () = assert!(
    DescriptorKind::StorageImage.as_i32() == VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
    "DescriptorKind::StorageImage must equal VK_DESCRIPTOR_TYPE_STORAGE_IMAGE"
);
const _: () = assert!(
    DescriptorKind::UniformBuffer.as_i32() == VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
    "DescriptorKind::UniformBuffer must equal VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER"
);
const _: () = assert!(
    DescriptorKind::StorageBuffer.as_i32() == VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
    "DescriptorKind::StorageBuffer must equal VK_DESCRIPTOR_TYPE_STORAGE_BUFFER"
);

// ===========================================================================
// CSM Increment 0 contracts. The comparison sampler maps `CompareOp` `as_i32()` in
// `create_sampler` (the `VkSamplerCreateInfo.compareOp`); the configurable graphics
// pipeline maps `CullMode` `as_u32()` in `create_graphics_pipeline` (the
// `VkPipelineRasterizationStateCreateInfo.cullMode`) by a trivial cast — so their
// discriminants MUST equal the matching `VK_COMPARE_OP_*` / `VK_CULL_MODE_*`
// constants. These pin that equality; any drift breaks the build instead of writing
// the wrong compare op / cull mode. (The depth-array texture's `VK_IMAGE_VIEW_TYPE_*`
// view types are backend-only constants the agnostic surface never names, so they
// have no agnostic counterpart to assert.)
// ===========================================================================

// --- CompareOp `as_i32()` (mapped in `create_sampler`). ---
const _: () = assert!(
    CompareOp::Never.as_i32() == VK_COMPARE_OP_NEVER,
    "CompareOp::Never must equal VK_COMPARE_OP_NEVER"
);
const _: () = assert!(
    CompareOp::Less.as_i32() == VK_COMPARE_OP_LESS,
    "CompareOp::Less must equal VK_COMPARE_OP_LESS"
);
const _: () = assert!(
    CompareOp::LessOrEqual.as_i32() == VK_COMPARE_OP_LESS_OR_EQUAL,
    "CompareOp::LessOrEqual must equal VK_COMPARE_OP_LESS_OR_EQUAL"
);
const _: () = assert!(
    CompareOp::Always.as_i32() == VK_COMPARE_OP_ALWAYS,
    "CompareOp::Always must equal VK_COMPARE_OP_ALWAYS"
);

// --- CullMode `as_u32()` (mapped in `create_graphics_pipeline`). ---
const _: () = assert!(
    CullMode::None.as_u32() == VK_CULL_MODE_NONE,
    "CullMode::None must equal VK_CULL_MODE_NONE"
);
const _: () = assert!(
    CullMode::Front.as_u32() == VK_CULL_MODE_FRONT_BIT,
    "CullMode::Front must equal VK_CULL_MODE_FRONT_BIT"
);
const _: () = assert!(
    CullMode::Back.as_u32() == VK_CULL_MODE_BACK_BIT,
    "CullMode::Back must equal VK_CULL_MODE_BACK_BIT"
);

// ===========================================================================
// HW-RT rung R2a-1 — acceleration-structure FFI ABI contracts (gated `hwrt`).
//
// Every `accel_ffi` struct a driver reads (build-info / geometry / create-info) or
// WRITES (the size + property structs) MUST match the C ABI or the driver reads/writes
// out of bounds. These `size_of` / `align_of` / `offset_of!` asserts pin each; the
// ABI-CRITICAL one is `VkAccelerationStructureInstanceKHR` (a driver reads a `[Self]`
// array during the TLAS build) — 64 B, align 8, offsets 0/48/52/56 — plus the 48-B
// row-major bridge (`transform: [[f32;4];3]`) that lets `InstanceModelCol.rows` memcpy in
// at R2a-3. The whole block is `#[cfg(feature="hwrt")]` so a default build asserts nothing.
// ===========================================================================
#[cfg(feature = "hwrt")]
mod hwrt_accel {
    use core::mem::{align_of, offset_of, size_of};

    use crate::accel_ffi::{
        VkAccelerationStructureBuildGeometryInfoKHR, VkAccelerationStructureBuildRangeInfoKHR,
        VkAccelerationStructureBuildSizesInfoKHR, VkAccelerationStructureCreateInfoKHR,
        VkAccelerationStructureDeviceAddressInfoKHR, VkAccelerationStructureGeometryDataKHR,
        VkAccelerationStructureGeometryInstancesDataKHR, VkAccelerationStructureGeometryKHR,
        VkAccelerationStructureGeometryTrianglesDataKHR, VkAccelerationStructureInstanceKHR,
        VkAccelerationStructureKHR, VkBufferDeviceAddressInfo,
        VkPhysicalDeviceAccelerationStructureFeaturesKHR,
        VkPhysicalDeviceAccelerationStructurePropertiesKHR,
        VkPhysicalDeviceBufferDeviceAddressFeatures, VkPhysicalDeviceRayQueryFeaturesKHR,
    };

    // --- The ABI-CRITICAL instance struct: 64 B, align 8, offsets 0/48/52/56. ---
    const _: () = assert!(
        size_of::<VkAccelerationStructureInstanceKHR>() == 64,
        "VkAccelerationStructureInstanceKHR must be 64 bytes (the spec-fixed TLAS instance stride)"
    );
    const _: () = assert!(
        align_of::<VkAccelerationStructureInstanceKHR>() == 8,
        "VkAccelerationStructureInstanceKHR must be 8-byte aligned"
    );
    const _: () = assert!(
        offset_of!(VkAccelerationStructureInstanceKHR, transform) == 0,
        "transform must be at offset 0 (a driver reads the 3x4 affine there)"
    );
    const _: () = assert!(
        offset_of!(VkAccelerationStructureInstanceKHR, instance_custom_index_and_mask) == 48,
        "customIndex|mask word must be at offset 48 (immediately after the 48-B transform)"
    );
    const _: () = assert!(
        offset_of!(VkAccelerationStructureInstanceKHR, instance_sbt_offset_and_flags) == 52,
        "sbtOffset|flags word must be at offset 52"
    );
    const _: () = assert!(
        offset_of!(VkAccelerationStructureInstanceKHR, acceleration_structure_reference) == 56,
        "accelerationStructureReference (BLAS device address) must be at offset 56"
    );

    // --- The 48-B row-major bridge: the `transform` field IS `[[f32;4];3]` (48 B), so a
    //     `boyko_render::InstanceModelCol.rows` (also `[[f32;4];3]`, 48 B row-major) memcpy
    //     into it at R2a-3 is a direct 48-byte copy with NO transpose. `boyko_rhi_vulkan`
    //     cannot name `InstanceModelCol` (that would be an upward dependency cycle); the
    //     equal-size contract is pinned HERE on the layout type + re-pinned on the render
    //     side (`INSTANCE_MODEL_COL_BYTES == 48`). ---
    const _: () = assert!(
        size_of::<[[f32; 4]; 3]>() == 48,
        "the 3x4 row-major affine bridge (InstanceModelCol.rows <-> InstanceKHR.transform) must be 48 bytes"
    );
    const _: () = assert!(
        size_of::<[[f32; 4]; 3]>() == size_of::<[f32; 12]>(),
        "the transform is 12 contiguous f32 with no padding"
    );

    // --- The handle is a 64-bit non-dispatchable token. ---
    const _: () = assert!(size_of::<VkAccelerationStructureKHR>() == 8);
    const _: () = assert!(align_of::<VkAccelerationStructureKHR>() == 8);

    // --- Feature / property query structs (driver-written through the p_next chain). ---
    // `VkPhysicalDeviceBufferDeviceAddressFeatures`: sType(4)+pad(4)+pNext(8)+3 VkBool32(12)
    // → 28, rounded up to align 8 = 32.
    const _: () = assert!(size_of::<VkPhysicalDeviceBufferDeviceAddressFeatures>() == 32);
    const _: () = assert!(align_of::<VkPhysicalDeviceBufferDeviceAddressFeatures>() == 8);
    // `VkPhysicalDeviceAccelerationStructureFeaturesKHR`: head(16)+5 VkBool32(20) → 36 → 40.
    const _: () = assert!(size_of::<VkPhysicalDeviceAccelerationStructureFeaturesKHR>() == 40);
    const _: () = assert!(align_of::<VkPhysicalDeviceAccelerationStructureFeaturesKHR>() == 8);
    // `VkPhysicalDeviceRayQueryFeaturesKHR`: head(16)+1 VkBool32(4) → 20 → 24.
    const _: () = assert!(size_of::<VkPhysicalDeviceRayQueryFeaturesKHR>() == 24);
    const _: () = assert!(align_of::<VkPhysicalDeviceRayQueryFeaturesKHR>() == 8);
    // `VkPhysicalDeviceAccelerationStructurePropertiesKHR`: head(16)+3 u64(24)+4 u32(16)+
    // scratch-align u32(4) = 60 → padded to 64 (align 8). The scratch-align field the caps
    // query reads sits at offset 56 (16 head + 24 + 16 = 56).
    const _: () = assert!(size_of::<VkPhysicalDeviceAccelerationStructurePropertiesKHR>() == 64);
    const _: () = assert!(align_of::<VkPhysicalDeviceAccelerationStructurePropertiesKHR>() == 8);
    const _: () = assert!(
        offset_of!(
            VkPhysicalDeviceAccelerationStructurePropertiesKHR,
            min_acceleration_structure_scratch_offset_alignment
        ) == 56
    );

    // --- Geometry / build / create / size / address structs. ---
    // TrianglesData: sType(4)+pad(4)+pNext(8)+format(4)+pad(4)+vertexData(8)+stride(8)+
    // maxVertex(4)+indexType(4)+indexData(8)+transformData(8) = 64.
    const _: () = assert!(size_of::<VkAccelerationStructureGeometryTrianglesDataKHR>() == 64);
    const _: () = assert!(align_of::<VkAccelerationStructureGeometryTrianglesDataKHR>() == 8);
    // InstancesData: sType(4)+pad(4)+pNext(8)+arrayOfPointers(4)+pad(4)+data(8) = 32.
    const _: () = assert!(size_of::<VkAccelerationStructureGeometryInstancesDataKHR>() == 32);
    const _: () = assert!(align_of::<VkAccelerationStructureGeometryInstancesDataKHR>() == 8);
    // The union is as large as its largest arm (the 64-B triangles struct), align 8.
    const _: () = assert!(size_of::<VkAccelerationStructureGeometryDataKHR>() == 64);
    const _: () = assert!(align_of::<VkAccelerationStructureGeometryDataKHR>() == 8);
    // GeometryKHR: sType(4)+pad(4)+pNext(8)+geometryType(4)+pad(4)+geometry(64)+flags(4)+
    // pad(4) = 96; the union `geometry` starts at offset 24.
    const _: () = assert!(size_of::<VkAccelerationStructureGeometryKHR>() == 96);
    const _: () = assert!(align_of::<VkAccelerationStructureGeometryKHR>() == 8);
    const _: () = assert!(offset_of!(VkAccelerationStructureGeometryKHR, geometry) == 24);
    // BuildGeometryInfo: sType(4)+pad(4)+pNext(8)+type(4)+flags(4)+mode(4)+pad(4)+src(8)+
    // dst(8)+geomCount(4)+pad(4)+pGeom(8)+ppGeom(8)+scratch(8) = 80; scratch at offset 72.
    const _: () = assert!(size_of::<VkAccelerationStructureBuildGeometryInfoKHR>() == 80);
    const _: () = assert!(align_of::<VkAccelerationStructureBuildGeometryInfoKHR>() == 8);
    const _: () =
        assert!(offset_of!(VkAccelerationStructureBuildGeometryInfoKHR, scratch_data) == 72);
    // BuildRangeInfo: four u32 = 16, align 4 (a driver reads an array of these).
    const _: () = assert!(size_of::<VkAccelerationStructureBuildRangeInfoKHR>() == 16);
    const _: () = assert!(align_of::<VkAccelerationStructureBuildRangeInfoKHR>() == 4);
    // BuildSizesInfo (driver-WRITTEN): sType(4)+pad(4)+pNext(8)+3 VkDeviceSize(24) = 40.
    const _: () = assert!(size_of::<VkAccelerationStructureBuildSizesInfoKHR>() == 40);
    const _: () = assert!(align_of::<VkAccelerationStructureBuildSizesInfoKHR>() == 8);
    const _: () = assert!(
        offset_of!(
            VkAccelerationStructureBuildSizesInfoKHR,
            acceleration_structure_size
        ) == 16
    );
    // CreateInfo: sType(4)+pad(4)+pNext(8)+createFlags(4)+pad(4)+buffer(8)+offset(8)+size(8)+
    // type(4)+pad(4)+deviceAddress(8) = 64.
    const _: () = assert!(size_of::<VkAccelerationStructureCreateInfoKHR>() == 64);
    const _: () = assert!(align_of::<VkAccelerationStructureCreateInfoKHR>() == 8);
    // DeviceAddressInfo: sType(4)+pad(4)+pNext(8)+accelStruct(8) = 24.
    const _: () = assert!(size_of::<VkAccelerationStructureDeviceAddressInfoKHR>() == 24);
    const _: () = assert!(align_of::<VkAccelerationStructureDeviceAddressInfoKHR>() == 8);
    // BufferDeviceAddressInfo: sType(4)+pad(4)+pNext(8)+buffer(8) = 24.
    const _: () = assert!(size_of::<VkBufferDeviceAddressInfo>() == 24);
    const _: () = assert!(align_of::<VkBufferDeviceAddressInfo>() == 8);

    // --- R2a-2: the DEVICE_ADDRESS alloc-flag chain struct (a driver reads it through the
    //     `VkMemoryAllocateInfo.p_next` chain during `vkAllocateMemory`). sType(4)+pad(4)+
    //     pNext(8)+flags(4)+deviceMask(4) = 24, align 8; flags@16, deviceMask@20. ---
    use crate::ffi::VkMemoryAllocateFlagsInfo;
    const _: () = assert!(size_of::<VkMemoryAllocateFlagsInfo>() == 24);
    const _: () = assert!(align_of::<VkMemoryAllocateFlagsInfo>() == 8);
    const _: () = assert!(offset_of!(VkMemoryAllocateFlagsInfo, p_next) == 8);
    const _: () = assert!(offset_of!(VkMemoryAllocateFlagsInfo, flags) == 16);
    const _: () = assert!(offset_of!(VkMemoryAllocateFlagsInfo, device_mask) == 20);
}
