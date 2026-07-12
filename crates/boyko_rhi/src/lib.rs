//! `boyko_rhi` — the backend-agnostic Render Hardware Interface trait surface.
//!
//! This crate is the in-house, **FFI-free** hal layer (wgpu-hal-shaped): an
//! umbrella [`RhiApi`] trait with associated owned-resource types, separate
//! operational traits ([`RhiDevice`], [`RhiQueue`], [`RhiCommandEncoder`]),
//! thin backend-agnostic enums/descriptors, and a generational handle registry
//! ([`ResourceRegistry`]). Backends (Vulkan today; DX12/Metal later) implement
//! these traits over their own owned resources via **static dispatch** — every
//! call monomorphizes to a direct, non-virtual call, so there is zero abstraction
//! overhead vs the backend's inherent methods. `RhiApi` is intentionally **not**
//! object-safe; there is no `dyn`, `Box`, or `HashMap` anywhere in the crate.
//!
//! # Scope (Phase 1): headless compute only
//!
//! Phase 1 abstracts exactly the headless-compute path — device, sub-allocated
//! buffers, host-visible mapping, compute pipeline from SPIR-V, command encoding
//! (begin/end/bind/dispatch/**buffer** barrier), submit + fence. The on-screen
//! path (`Surface`/`Swapchain`/semaphores/present submit + image-layout barriers)
//! stays concrete in the backend and is a genuine **Phase-2-3 seam**; SDF
//! textures, graphics pipelines, bind groups, and indirect dispatch are
//! **Phase-6+ seams**. Seam associated types are declared now (unbounded) and
//! seam methods carry `#[cold]` default-erroring stubs, so the trait surface
//! stays stable across phases with no ABI break when a feature lands.
//!
//! # Dependencies
//!
//! `boyko_utils` **only** (for [`Slot`](boyko_utils::identifiers::slot::Slot) /
//! `SparseSlotMap` / `Generation`). The `boyko_ecs` dependency and core's
//! `DeviceColumnHandle(u64)` newtype land in Phase 4; for now the registry uses
//! its own typed `Slot`-based handles and exposes the
//! [`slot_to_u64`]/[`u64_to_slot`] packing bridge Phase 4 will wire to core.

pub mod api;
pub mod descriptor;
pub mod device;
pub mod encoder;
pub mod enums;
pub mod error;
pub mod handle;
pub mod queue;

pub use api::RhiApi;
pub use descriptor::{
    AsBuildEntry, AsBuildSizes, AsGeometryDesc, AsIndexType, AsKind, BarrierDesc, BufferBarrier,
    BufferCopy,
    BufferDesc, BufferImageCopy, ComputePipelineDesc, DepthAttachment, DepthBias,
    GraphicsPipelineDesc, ImageBarrierDesc, ImageBlitDesc, ImageSubresourceRange, QueryPoolDesc,
    RenderArea, RenderingAttachment, RenderingDesc, SpecConstant, VertexAttribute,
    VertexBufferLayout, Viewport,
};
pub use device::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry,
    MAX_BIND_GROUP_BINDINGS, MipMode, RhiDevice, SamplerDesc, TextureDesc,
};
pub use encoder::RhiCommandEncoder;
pub use enums::{
    AddressMode, BarrierAccess, BarrierStage, BlendFactor, BlendOp, BlendState, BufferUsage,
    CompareOp, CullMode, DescriptorKind, Filter, Format, ImageAspect, ImageLayout, ImageUsage,
    IndexType, LoadOp, MemoryLocation, PrimitiveTopology, ShaderStage, StoreOp, TextureDimension,
    TimestampStage, VertexFormat,
};
pub use error::RhiError;
pub use handle::{
    slot_to_u64, u64_to_slot, BufferHandle, ComputePipelineHandle, FenceHandle, ResourceRegistry,
    ShaderHandle,
};
pub use queue::RhiQueue;
