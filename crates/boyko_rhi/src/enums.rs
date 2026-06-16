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
    /// `VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`.
    pub const COMPUTE_SHADER: BarrierStage = BarrierStage(0x0000_0800);
    /// `VK_PIPELINE_STAGE_TRANSFER_BIT`.
    pub const TRANSFER: BarrierStage = BarrierStage(0x0000_1000);

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
