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

use boyko_rhi::enums::{BarrierAccess, BarrierStage, BufferUsage, ShaderStage};

use crate::ffi::{
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT,
    VK_ACCESS_TRANSFER_WRITE_BIT, VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
    VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, VK_BUFFER_USAGE_TRANSFER_DST_BIT,
    VK_BUFFER_USAGE_TRANSFER_SRC_BIT, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
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
