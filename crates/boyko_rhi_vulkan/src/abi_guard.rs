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
    BarrierAccess, BarrierStage, BufferUsage, Format, ImageAspect, ImageLayout, ImageUsage, LoadOp,
    PrimitiveTopology, ShaderStage, StoreOp, TextureDimension,
};

use crate::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT,
    VK_ACCESS_TRANSFER_READ_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, VK_ATTACHMENT_LOAD_OP_CLEAR,
    VK_ATTACHMENT_LOAD_OP_LOAD, VK_ATTACHMENT_STORE_OP_STORE, VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
    VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, VK_BUFFER_USAGE_TRANSFER_DST_BIT,
    VK_BUFFER_USAGE_TRANSFER_SRC_BIT, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT, VK_FORMAT_B8G8R8A8_UNORM,
    VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_UNDEFINED, VK_IMAGE_ASPECT_COLOR_BIT,
    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_GENERAL,
    VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_TYPE_2D, VK_IMAGE_TYPE_3D,
    VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT, VK_IMAGE_USAGE_SAMPLED_BIT, VK_IMAGE_USAGE_STORAGE_BIT,
    VK_IMAGE_USAGE_TRANSFER_DST_BIT, VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
    VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
    VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST, VK_SHADER_STAGE_COMPUTE_BIT,
    VK_SHADER_STAGE_FRAGMENT_BIT, VK_SHADER_STAGE_VERTEX_BIT,
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
