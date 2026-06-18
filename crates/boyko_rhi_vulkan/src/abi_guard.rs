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
//! Every bit the backend identity-casts (`rhi_impl.rs::create_buffer`,
//! `push_constants`, `pipeline_barrier`) is covered.

use boyko_rhi::enums::{
    AddressMode, BarrierAccess, BarrierStage, BufferUsage, DescriptorKind, Filter, Format,
    ImageAspect, ImageLayout, ImageUsage, IndexType, LoadOp, PrimitiveTopology, ShaderStage,
    StoreOp, TextureDimension, VertexFormat,
};

use crate::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT,
    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT,
    VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
    VK_ATTACHMENT_LOAD_OP_CLEAR, VK_ATTACHMENT_LOAD_OP_LOAD, VK_ATTACHMENT_STORE_OP_STORE,
    VK_BUFFER_USAGE_INDEX_BUFFER_BIT, VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
    VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, VK_BUFFER_USAGE_TRANSFER_DST_BIT,
    VK_BUFFER_USAGE_TRANSFER_SRC_BIT, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT,
    VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
    VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
    VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, VK_FILTER_LINEAR,
    VK_FILTER_NEAREST, VK_FORMAT_B8G8R8A8_UNORM,
    VK_FORMAT_D32_SFLOAT, VK_FORMAT_R32G32B32A32_SFLOAT, VK_FORMAT_R32G32B32_SFLOAT,
    VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_UNDEFINED, VK_IMAGE_ASPECT_COLOR_BIT,
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
// reinterpret in `set_viewport`) is asserted at the cast site in `rhi_impl.rs`.
// ===========================================================================

// --- PrimitiveTopology `as_i32()` (mapped in `create_graphics_pipeline`). ---
const _: () = assert!(
    PrimitiveTopology::TriangleList.as_i32() == VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
    "PrimitiveTopology::TriangleList must equal VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST"
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

// --- IndexType `as_i32()` (mapped in `bind_index_buffer`). ---
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
