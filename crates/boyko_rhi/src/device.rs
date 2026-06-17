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
use crate::enums::{AddressMode, Filter, Format, ImageUsage, ShaderStage, TextureDimension};
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

/// Parameters for [`RhiDevice::create_sampler`] (Phase-6 S0 rung 5).
///
/// `#[repr(C)]` POD with an explicit field order (the two `i32` `VkFilter` seam
/// fields, then the `i32` `VkSamplerAddressMode`) so a backend reads it without
/// depending on Rust's default field reordering. Rung 5 picks
/// [`Filter::Nearest`] + [`AddressMode::ClampToEdge`] — the simplest deterministic
/// 1:1 sample (one source texel per sampled texel, an out-of-range UV clamps to
/// the edge). The same address mode is applied to all three coordinate axes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerDesc {
    /// The magnification filter (sampling a texel larger than one source texel).
    pub mag_filter: Filter,
    /// The minification filter (sampling a texel smaller than one source texel).
    pub min_filter: Filter,
    /// The address mode applied to every texture-coordinate axis.
    pub address_mode: AddressMode,
}

impl Default for SamplerDesc {
    /// The deterministic 1:1 rung-5 default: nearest mag/min + clamp-to-edge.
    #[inline]
    fn default() -> Self {
        SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
        }
    }
}

/// Parameters for [`RhiDevice::create_bind_group_layout`] (Phase-6 S0 rung 5).
///
/// Declares `binding_count` consecutive COMBINED_IMAGE_SAMPLER bindings at
/// `(set 0, binding 0..binding_count)`, all visible to the `stage` shader stage(s).
/// Rung 5 uses [`ShaderStage::FRAGMENT`] + `binding_count == 1` (one sampled
/// texture). Rung 6's deferred-lighting pass uses `binding_count == 2` — the two
/// G-buffer inputs (albedo + normal) sampled together in one fragment shader. The
/// backend caps the count at a small fixed maximum (a `debug_assert!` traps an
/// over-count).
#[derive(Debug, Clone, Copy)]
pub struct BindGroupLayoutDesc {
    /// The shader stage(s) every combined-image-sampler binding is visible to.
    pub stage: ShaderStage,
    /// The number of consecutive COMBINED_IMAGE_SAMPLER bindings (rung 5: `1`;
    /// rung 6 deferred lighting: `2`). Must be `>= 1` and within the backend cap.
    pub binding_count: u32,
}

/// One `(texture view, sampler)` entry written into a [`BindGroupDesc`]'s descriptor
/// set at the entry's positional binding index (Phase-6 S0 rung 6).
///
/// Borrows the texture + sampler for the `create_bind_group` call only — but the
/// resulting bind group retains them BY RAW HANDLE (see [`BindGroupDesc`]'s caller
/// contract). The texture MUST be in
/// [`crate::enums::ImageLayout::ShaderReadOnlyOptimal`] before a draw samples it.
pub struct BindGroupEntry<'a, A: RhiApi> {
    /// The texture whose image view is bound as the sampled image at this binding.
    pub texture: &'a A::Texture,
    /// The sampler bound alongside the image (the COMBINED part).
    pub sampler: &'a A::Sampler,
}

/// Parameters for [`RhiDevice::create_bind_group`] (Phase-6 S0 rung 5/6).
///
/// Carries the [`RhiDevice::create_bind_group_layout`] layout the set is allocated
/// against plus one [`BindGroupEntry`] per COMBINED_IMAGE_SAMPLER binding, written
/// into bindings `0..entries.len()` in slice order. Rung 5 supplies one entry; rung 6
/// deferred lighting supplies two (albedo + normal). `entries.len()` MUST equal the
/// layout's `binding_count`. Every entry's texture MUST be in
/// [`crate::enums::ImageLayout::ShaderReadOnlyOptimal`] (transitioned via
/// [`crate::encoder::RhiCommandEncoder::image_barrier`]) before a draw samples the
/// bound set, or the validation layer faults at draw time. The `'a` lifetime
/// borrows the layout + entries for the `create_bind_group` call only —
/// **but the resulting bind group retains each texture's image view and sampler
/// BY RAW HANDLE in its descriptor set.** CALLER CONTRACT: every texture and sampler
/// MUST outlive every submission that binds this group; dropping any before the
/// binding submission completes is use-after-free of a destroyed view/sampler
/// (caught by the validation layer, not the Rust type system — the compile-time
/// lifetime tie is deferred to Phase 2-3, plan F1).
pub struct BindGroupDesc<'a, A: RhiApi> {
    /// The layout the descriptor set is allocated + written against.
    pub layout: &'a A::BindGroupLayout,
    /// One `(texture, sampler)` entry per binding, in binding order; its length must
    /// equal the layout's `binding_count`.
    pub entries: &'a [BindGroupEntry<'a, A>],
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

    /// Creates a sampler (Phase-6 S0 rung 5: a `VkSampler` with the desc's
    /// mag/min filter + address mode — rung 5 uses nearest + clamp-to-edge for a
    /// deterministic 1:1 sample).
    ///
    /// The default body is `#[cold] #[inline(never)]` and errors `Unsupported`; a
    /// backend with a sampler path (Vulkan) overrides it. Keeps the trait ABI
    /// stable for a backend (e.g. the Mock) without one.
    #[cold]
    #[inline(never)]
    fn create_sampler(&self, _desc: &SamplerDesc) -> Result<A::Sampler, Self::Error> {
        Err(RhiError::unsupported("create_sampler").into())
    }

    /// Destroys `sampler`, consuming it (Phase-6 S0 rung 5).
    ///
    /// The default body drops the value (a no-op for a backend whose `Sampler` is
    /// zero-sized, e.g. the Mock); a backend whose sampler owns a GPU object
    /// (Vulkan) overrides it. Keeps the trait ABI stable.
    ///
    /// # Safety
    /// The GPU must no longer be using `sampler` (a submission referencing it has
    /// completed — fence-waited or `wait_idle`'d). The by-value move guarantees it
    /// is destroyed at most once.
    #[cold]
    #[inline(never)]
    unsafe fn destroy_sampler(&self, sampler: A::Sampler) {
        // Default seam: drop the value. A zero-sized `Sampler` (Mock) drops to a
        // no-op; a backend with a GPU-owned sampler object overrides this.
        drop(sampler);
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

    /// Creates a bind-group layout (Phase-6 S0 rung 5/6: a `VkDescriptorSetLayout`
    /// with `desc.binding_count` COMBINED_IMAGE_SAMPLER bindings at
    /// `(set 0, binding 0..binding_count)` at the desc's stage). Supersedes the fixed
    /// compute descriptor layout for the graphics sampling path.
    ///
    /// The default body is `#[cold] #[inline(never)]` and errors `Unsupported`; a
    /// backend with a descriptor path (Vulkan) overrides it.
    #[cold]
    #[inline(never)]
    fn create_bind_group_layout(
        &self,
        _desc: &BindGroupLayoutDesc,
    ) -> Result<A::BindGroupLayout, Self::Error> {
        Err(RhiError::unsupported("create_bind_group_layout").into())
    }

    /// Destroys `layout`, consuming it (Phase-6 S0 rung 5).
    ///
    /// The default body drops the value (a no-op for a backend whose
    /// `BindGroupLayout` is zero-sized, e.g. the Mock); a backend whose layout
    /// owns a GPU object (Vulkan) overrides it. Keeps the trait ABI stable.
    ///
    /// # Safety
    /// No bind group / pipeline still referencing `layout` is in flight, and it is
    /// destroyed exactly once (the move enforces the latter).
    #[cold]
    #[inline(never)]
    unsafe fn destroy_bind_group_layout(&self, layout: A::BindGroupLayout) {
        // Default seam: drop the value. A zero-sized `BindGroupLayout` (Mock) drops
        // to a no-op; a backend with a GPU-owned set-layout overrides this.
        drop(layout);
    }

    /// Creates a bind group (Phase-6 S0 rung 5/6: a `VkDescriptorPool` + a single
    /// `VkDescriptorSet` allocated against `desc.layout` and written with one
    /// `(texture view, sampler)` COMBINED_IMAGE_SAMPLER per `desc.entries` entry, in
    /// `SHADER_READ_ONLY_OPTIMAL`).
    ///
    /// The default body is `#[cold] #[inline(never)]` and errors `Unsupported`; a
    /// backend with a descriptor path (Vulkan) overrides it.
    #[cold]
    #[inline(never)]
    fn create_bind_group(
        &self,
        _desc: &BindGroupDesc<A>,
    ) -> Result<A::BindGroup, Self::Error> {
        Err(RhiError::unsupported("create_bind_group").into())
    }

    /// Destroys `group`, consuming it (Phase-6 S0 rung 5).
    ///
    /// The default body drops the value (a no-op for a backend whose `BindGroup`
    /// is zero-sized, e.g. the Mock); a backend whose bind group owns a GPU object
    /// (Vulkan: a `VkDescriptorPool`) overrides it. Keeps the trait ABI stable.
    ///
    /// # Safety
    /// No submission using `group` is pending (the GPU is fence-waited /
    /// `wait_idle`'d), and it is destroyed exactly once (the move enforces the
    /// latter).
    #[cold]
    #[inline(never)]
    unsafe fn destroy_bind_group(&self, group: A::BindGroup) {
        // Default seam: drop the value. A zero-sized `BindGroup` (Mock) drops to a
        // no-op; a backend with a GPU-owned descriptor pool overrides this.
        drop(group);
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
