//! The [`RhiDevice`] operational trait: resource lifecycle + sync.
//!
//! Foundation-now methods (buffer/shader/pipeline/fence/encoder create+destroy,
//! mapped-pointer, fence wait/reset, `wait_idle`) are fully specified and map
//! directly onto the existing Slice-0 Vulkan code. Deferred-seam methods carry a
//! `#[cold] #[inline(never)]` default body returning `Unsupported` (plan D7), so
//! a backend overrides them only when the feature lands — the trait stays ABI-
//! stable across phases.

use crate::api::RhiApi;
use crate::descriptor::{BufferDesc, ComputePipelineDesc, GraphicsPipelineDesc};
use crate::enums::{Format, ImageUsage, TextureDimension};
use crate::error::RhiError;

/// Parameters for [`RhiDevice::create_texture`] (Phase-6 S0 graphics surface).
///
/// `#[repr(C)]` POD with an explicit field order (dimension + format are the
/// `i32` FFI seam per `enums.rs`, the extent + usage follow) so a backend can read
/// it without depending on Rust's default field reordering. Rung 1 creates a 2D
/// color image with `COLOR_ATTACHMENT | TRANSFER_SRC` usage (clear → readback);
/// `D3` + `STORAGE` are reserved for the deferred SDF storage image.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureDesc {
    /// Width in texels (`> 0`).
    pub width: u32,
    /// Height in texels (`> 0`).
    pub height: u32,
    /// Depth in texels for a [`TextureDimension::D3`] image; `1` for a 2D image.
    pub depth: u32,
    /// The texel format.
    pub format: Format,
    /// 2D or 3D.
    pub dimension: TextureDimension,
    /// The usage bits the image must support.
    pub usage: ImageUsage,
}

/// Minimal placeholder descriptor for the Phase-6+ sampler seam (plan D7).
#[derive(Debug, Clone, Copy, Default)]
pub struct SamplerDesc {
    /// Reserved; the sampler seam's fields land in Phase 6+.
    pub _reserved: (),
}

/// Minimal placeholder descriptor for the Phase-6+ bind-group-layout seam.
#[derive(Debug, Clone, Copy, Default)]
pub struct BindGroupLayoutDesc {
    /// Reserved; the bind-group-layout seam's fields land in Phase 6+.
    pub _reserved: (),
}

/// Minimal placeholder descriptor for the Phase-6+ bind-group seam.
#[derive(Debug, Clone, Copy, Default)]
pub struct BindGroupDesc {
    /// Reserved; the bind-group seam's fields land in Phase 6+.
    pub _reserved: (),
}

/// The logical device: creates and destroys backend resources, maps buffers,
/// builds pipelines, and provides the CPU↔GPU sync primitives.
///
/// `destroy_*` methods are `unsafe`: the caller must guarantee the GPU is no
/// longer using the resource (fence-waited / `wait_idle`'d) and that the
/// resource is destroyed exactly once — the by-value move already encodes the
/// "exactly once" half in the type system (plan D2).
///
/// # Lifetime contract (plan F1 / RL-1)
///
/// An `A::Buffer`/`A::Fence`/etc. produced by this device, and the `&self` device
/// itself, are **not** tied by a compile-time lifetime to the originating context.
/// The originating device/context MUST still be alive when any `destroy_*` (or a
/// `RhiQueue::submit` referencing these resources) runs — destroying or submitting
/// after the context is dropped is **undefined behavior** (backend resources hold
/// raw pointers into the context's fn-table). This is the accepted plan-D2
/// trade-off; the structural `'ctx` lifetime parameter is **deferred to Phase 2-3**
/// (the on-screen-in-trait work), not added now.
pub trait RhiDevice<A: RhiApi> {
    /// One unified per-backend error type (plan D4). The bound is `From<RhiError>`
    /// **only** (one direction) so a seam stub can `Err(RhiError::…​.into())`; the
    /// agnostic projection is a hand-written `impl From<BackendError> for RhiError`
    /// in the backend, avoiding the reflexive-blanket coherence wall.
    type Error: core::fmt::Debug + From<RhiError>;

    // ===== FOUNDATION-NOW =====

    /// Creates a buffer per `desc`.
    fn create_buffer(&self, desc: &BufferDesc) -> Result<A::Buffer, Self::Error>;

    /// Destroys `buffer`, consuming it.
    ///
    /// # Safety
    /// The GPU must no longer be using `buffer` (a submission referencing it has
    /// completed — fence-waited or `wait_idle`'d). The by-value move guarantees
    /// it is destroyed at most once.
    unsafe fn destroy_buffer(&self, buffer: A::Buffer);

    /// Returns the persistently-mapped host pointer for a host-visible buffer, or
    /// `None` if the buffer is not host-mappable.
    fn buffer_mapped_ptr(&self, buffer: &A::Buffer) -> Option<core::ptr::NonNull<u8>>;

    /// Creates a shader module from SPIR-V words.
    fn create_shader_module(&self, spirv: &[u32]) -> Result<A::ShaderModule, Self::Error>;

    /// Destroys `module`, consuming it.
    ///
    /// # Safety
    /// No pipeline still referencing `module` is in flight, and it is destroyed
    /// exactly once (the move enforces the latter).
    unsafe fn destroy_shader_module(&self, module: A::ShaderModule);

    /// Creates a compute pipeline per `desc`.
    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<A>,
    ) -> Result<A::ComputePipeline, Self::Error>;

    /// Destroys `pipeline`, consuming it.
    ///
    /// # Safety
    /// No submission using `pipeline` is pending, and it is destroyed exactly
    /// once (the move enforces the latter).
    unsafe fn destroy_compute_pipeline(&self, pipeline: A::ComputePipeline);

    /// Creates a fence, initially signaled iff `signaled`.
    fn create_fence(&self, signaled: bool) -> Result<A::Fence, Self::Error>;

    /// Destroys `fence`, consuming it.
    ///
    /// # Safety
    /// `fence` is not pending (no in-flight submission will signal it), and it is
    /// destroyed exactly once (the move enforces the latter).
    unsafe fn destroy_fence(&self, fence: A::Fence);

    /// Waits for `fence` to be signaled, up to `timeout_ns` nanoseconds.
    fn wait_fence(&self, fence: &A::Fence, timeout_ns: u64) -> Result<(), Self::Error>;

    /// Resets `fence` to the unsignaled state.
    fn reset_fence(&self, fence: &A::Fence) -> Result<(), Self::Error>;

    /// Creates a command encoder (owns its command pool + buffer + descriptor
    /// pool + set, per plan Q1).
    fn create_command_encoder(&self) -> Result<A::CommandEncoder, Self::Error>;

    /// Destroys `enc`, consuming it.
    ///
    /// # Safety
    /// `enc`'s last submission has completed (not pending), and it is destroyed
    /// exactly once (the move enforces the latter).
    unsafe fn destroy_command_encoder(&self, enc: A::CommandEncoder);

    /// Blocks until the device is idle (`vkDeviceWaitIdle`). The belt-and-braces
    /// teardown sync the registry's `destroy_all` calls first (plan W4).
    fn wait_idle(&self) -> Result<(), Self::Error>;

    // ===== DEFERRED SEAM (Phase 5/6+) — default-erroring stubs =====

    /// Creates a texture (Phase-6 S0: a 2D/3D color image + view + bound memory).
    #[cold]
    #[inline(never)]
    fn create_texture(&self, _desc: &TextureDesc) -> Result<A::Texture, Self::Error> {
        Err(RhiError::unsupported("create_texture").into())
    }

    /// Destroys `texture`, consuming it (Phase-6 S0).
    ///
    /// The default body drops the value (a no-op for a backend whose `Texture` is
    /// zero-sized, e.g. the Mock); a backend whose texture owns GPU objects (Vulkan)
    /// overrides it to tear them down. This keeps the trait ABI stable.
    ///
    /// # Safety
    /// The GPU must no longer be using `texture` (a submission referencing it has
    /// completed — fence-waited or `wait_idle`'d). The by-value move guarantees it
    /// is destroyed at most once.
    #[cold]
    #[inline(never)]
    unsafe fn destroy_texture(&self, texture: A::Texture) {
        // Default seam: drop the value. A zero-sized `Texture` (Mock) drops to a
        // no-op; a backend with GPU-owned image objects overrides this.
        drop(texture);
    }

    /// Creates a sampler. Seam: Phase 6+.
    #[cold]
    #[inline(never)]
    fn create_sampler(&self, _desc: &SamplerDesc) -> Result<A::Sampler, Self::Error> {
        Err(RhiError::unsupported("create_sampler").into())
    }

    /// Creates a graphics pipeline (Phase-6 S0 rung 2: a Vulkan 1.3
    /// dynamic-rendering pipeline — vertex + fragment stages, an empty pipeline
    /// layout, dynamic viewport/scissor, single color attachment whose format is
    /// declared in `desc`).
    ///
    /// The default body is `#[cold] #[inline(never)]` and errors `Unsupported`; a
    /// backend with a graphics path (Vulkan) overrides it. Keeps the trait ABI
    /// stable for a backend (e.g. the Mock) without one.
    #[cold]
    #[inline(never)]
    fn create_graphics_pipeline(
        &self,
        _desc: &GraphicsPipelineDesc<A>,
    ) -> Result<A::GraphicsPipeline, Self::Error> {
        Err(RhiError::unsupported("create_graphics_pipeline").into())
    }

    /// Destroys `pipeline`, consuming it (Phase-6 S0 rung 2).
    ///
    /// The default body drops the value (a no-op for a backend whose
    /// `GraphicsPipeline` is zero-sized, e.g. the Mock); a backend whose pipeline
    /// owns GPU objects (Vulkan) overrides it. Keeps the trait ABI stable.
    ///
    /// # Safety
    /// No submission using `pipeline` is pending (the GPU is fence-waited /
    /// `wait_idle`'d), and it is destroyed exactly once (the by-value move enforces
    /// the latter).
    #[cold]
    #[inline(never)]
    unsafe fn destroy_graphics_pipeline(&self, pipeline: A::GraphicsPipeline) {
        // Default seam: drop the value. A zero-sized `GraphicsPipeline` (Mock)
        // drops to a no-op; a backend with GPU-owned pipeline objects overrides it.
        drop(pipeline);
    }

    /// Creates a bind-group layout. Seam: Phase 6+ (supersedes the fixed compute
    /// descriptor layout).
    #[cold]
    #[inline(never)]
    fn create_bind_group_layout(
        &self,
        _desc: &BindGroupLayoutDesc,
    ) -> Result<A::BindGroupLayout, Self::Error> {
        Err(RhiError::unsupported("create_bind_group_layout").into())
    }

    /// Creates a bind group. Seam: Phase 6+.
    #[cold]
    #[inline(never)]
    fn create_bind_group(&self, _desc: &BindGroupDesc) -> Result<A::BindGroup, Self::Error> {
        Err(RhiError::unsupported("create_bind_group").into())
    }

    /// Maps a non-coherent buffer's range to a host pointer. Seam: Phase 5
    /// (device-local `GpuColumn` staging — host-coherent mapping does not extend
    /// to device-local memory).
    #[cold]
    #[inline(never)]
    fn map_buffer(&self, _buffer: &A::Buffer) -> Result<core::ptr::NonNull<u8>, Self::Error> {
        Err(RhiError::unsupported("map_buffer").into())
    }

    /// Unmaps + flushes a previously `map_buffer`'d range. Seam: Phase 5.
    #[cold]
    #[inline(never)]
    fn unmap_buffer(&self, _buffer: &A::Buffer) -> Result<(), Self::Error> {
        Err(RhiError::unsupported("unmap_buffer").into())
    }
}
