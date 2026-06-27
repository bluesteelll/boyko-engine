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
use crate::enums::{
    AddressMode, CompareOp, DescriptorKind, Filter, Format, ImageUsage, ShaderStage,
    TextureDimension,
};
use crate::error::RhiError;

/// The maximum number of bindings a single bind group / its layout declares
/// (Render P1a). The backend mirrors this as its fixed-capacity inline-array size,
/// so the descriptor create path never heap-allocates.
///
/// Sized for the lighting L0/L1 inputs on a SINGLE descriptor set with headroom:
/// the deferred resolve grows 6 → 7 (L0a `+light_table`) → 8 (L0b `+gViewT`) → 10
/// (L1 `+cluster_grid` `+light_index`), and the marcher vocab set grows 8 → 9 (L0b
/// `+gViewT`). The SDF clip-map (M4) binds `N = brick::BRICK_LEVELS` levels × 2
/// resources (a pointer-grid SSBO + an atlas combined-image) at SEPARATE bindings
/// 9..=14 on top of the 0..=8 gbuffer bindings = 15 total, so the cap is 16 (N
/// levels × 2, +1 headroom). This is a self-imposed engine inline-array size, NOT a
/// hardware limit (NVIDIA Ampere / RTX 3060 `maxPerStageDescriptorStorageImages` /
/// `maxPerStageResources` are both 1048576, so 16 is safe). A `debug_assert!` traps
/// an over-count.
pub const MAX_BIND_GROUP_BINDINGS: usize = 16;

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
    /// The number of array layers in the image (CSM Increment 0). `1` (the default)
    /// is today's single-layer image with one full-subresource view — byte-identical
    /// to every existing texture. `> 1` (a `DEPTH`-format image) makes the backend
    /// create the image with `arrayLayers = N` plus a VIEW SET: one
    /// `VK_IMAGE_VIEW_TYPE_2D` per-layer RENDER view (each cascade renders into its
    /// own layer) and one `VK_IMAGE_VIEW_TYPE_2D_ARRAY` SAMPLE view (the resolve
    /// samples `float3(uv, layer)`). Capped at the backend's `MAX_CASCADES`.
    pub array_layers: u32,
}

impl Default for TextureDesc {
    /// A single-layer (`array_layers == 1`) texture — the byte-identical default for
    /// every non-array image. The extent/format/dimension/usage fields have no
    /// universal default and MUST be set by the caller; this impl exists so a caller
    /// can spread `..TextureDesc::default()` to pick up `array_layers: 1` (and so the
    /// CSM array texture is the only site that overrides it).
    #[inline]
    fn default() -> Self {
        TextureDesc {
            width: 1,
            height: 1,
            depth: 1,
            format: Format::Undefined,
            dimension: TextureDimension::D2,
            usage: ImageUsage::NONE,
            array_layers: 1,
        }
    }
}

/// The mip/LOD sampling mode (GUI P5b Decision T4-D).
///
/// `#[repr(i32)]` so the backend reads it as a small POD discriminant. P5b uses
/// [`Self::None`] exclusively: the MSDF atlas is upload-once at a single rasterized
/// `pixels_per_em` and the `screenPxRange` AA derivation assumes the base level is
/// sampled (a mipped read would corrupt the per-channel `median()`). Making the
/// no-mip intent a DECLARED, gate-backed field (rather than a backend-default
/// accident) is the Decision-T4-D fix; the variant set grows when a future texture
/// path needs mipmaps.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipMode {
    /// No mipmapping: the backend pins `mipmapMode = NEAREST`, `minLod = maxLod =
    /// 0.0`, so every sample reads the base level. The P5b MSDF-atlas requirement.
    None = 0,
}

/// Parameters for [`RhiDevice::create_sampler`] (Phase-6 S0 rung 5).
///
/// `#[repr(C)]` POD with an explicit field order (the two `i32` `VkFilter` seam
/// fields, then the `i32` `VkSamplerAddressMode`, then the `i32` [`MipMode`]) so a
/// backend reads it without depending on Rust's default field reordering. Rung 5
/// picks [`Filter::Nearest`] + [`AddressMode::ClampToEdge`] — the simplest
/// deterministic 1:1 sample (one source texel per sampled texel, an out-of-range UV
/// clamps to the edge). The same address mode is applied to all three coordinate
/// axes. The MSDF atlas (GUI P5b) overrides the filters to [`Filter::Linear`] while
/// keeping [`MipMode::None`] (bilinear, no mips — Decision T4-D).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerDesc {
    /// The magnification filter (sampling a texel larger than one source texel).
    pub mag_filter: Filter,
    /// The minification filter (sampling a texel smaller than one source texel).
    pub min_filter: Filter,
    /// The address mode applied to every texture-coordinate axis.
    pub address_mode: AddressMode,
    /// The mip/LOD mode (GUI P5b Decision T4-D). P5b uses [`MipMode::None`].
    pub mip: MipMode,
    /// The optional hardware depth-comparison op (CSM Increment 0). `None` (the
    /// default) leaves `compareEnable = VK_FALSE` — byte-identical to every existing
    /// sampler. `Some(op)` builds a COMPARISON sampler (`compareEnable = VK_TRUE`,
    /// `compareOp = op`) so a shadow-map PCF read returns the filtered pass/fail of
    /// `reference (op) stored_depth` rather than the raw depth; PCF uses
    /// [`CompareOp::LessOrEqual`].
    pub compare: Option<CompareOp>,
}

impl Default for SamplerDesc {
    /// The deterministic 1:1 rung-5 default: nearest mag/min + clamp-to-edge + no
    /// mips + no compare (the existing behavior, now an explicit [`MipMode::None`] +
    /// `compare: None`).
    #[inline]
    fn default() -> Self {
        SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
            compare: None,
        }
    }
}

/// One binding's declaration within a [`BindGroupLayoutDesc`] (Render P1a).
///
/// `#[repr(C)]` POD with an explicit field order so a backend reads it without
/// depending on Rust's default field reordering — it maps onto a
/// `VkDescriptorSetLayoutBinding`'s `(binding, descriptorType, descriptorCount,
/// stageFlags)`. `count` is `1` in P1a but is carried so a future bindless layout
/// can declare an array binding without an ABI break.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindGroupLayoutEntry {
    /// The binding index within set 0 (`layout(binding = N)`).
    pub binding: u32,
    /// The descriptor count at this binding (`1` in P1a; `> 1` for a future
    /// bindless array).
    pub count: u32,
    /// The kind of resource this binding holds.
    pub kind: DescriptorKind,
    /// The shader stage(s) this binding is visible to.
    pub stage: ShaderStage,
}

/// Parameters for [`RhiDevice::create_bind_group_layout`] (Render P1a).
///
/// Declares a `VkDescriptorSetLayout` from a slice of heterogeneous
/// [`BindGroupLayoutEntry`]s at `set 0` — generalizing the prior
/// COMBINED_IMAGE_SAMPLER-only form into the multi-resource descriptor vocabulary.
/// `entries.len()` must be in `1..=`[`MAX_BIND_GROUP_BINDINGS`] (the backend caps it
/// with a `debug_assert!`). The matching [`BindGroupDesc`] writes one
/// [`BindGroupEntry`] per layout entry, in the same order, and each
/// `BindGroupEntry`'s variant must match its layout entry's
/// [`BindGroupLayoutEntry::kind`].
///
/// The `'a` lifetime borrows the entry slice for the `create_bind_group_layout`
/// call only (the backend copies what it needs into the driver).
#[derive(Debug, Clone, Copy)]
pub struct BindGroupLayoutDesc<'a> {
    /// The heterogeneous bindings the layout declares at `set 0`, in binding order;
    /// `1..=`[`MAX_BIND_GROUP_BINDINGS`] entries.
    pub entries: &'a [BindGroupLayoutEntry],
}

/// One resource written into a [`BindGroupDesc`]'s descriptor set at the matching
/// layout entry's binding (Render P1a) — a tagged union over the five
/// [`DescriptorKind`]s.
///
/// Each variant borrows the backend resource(s) for the `create_bind_group` call
/// only — but the resulting bind group retains them BY RAW HANDLE in its descriptor
/// set (see [`BindGroupDesc`]'s caller contract). The variant MUST match the
/// corresponding [`BindGroupLayoutEntry::kind`] (a `debug_assert!` traps a
/// mismatch). A sampled image (`CombinedImage`/`SampledImage`) MUST be in
/// [`crate::enums::ImageLayout::ShaderReadOnlyOptimal`] and a `StorageImage` in
/// [`crate::enums::ImageLayout::General`] before a draw/dispatch accesses it.
pub enum BindGroupEntry<'a, A: RhiApi> {
    /// A `STORAGE_IMAGE` — a read/write image bound by view (no sampler).
    StorageImage {
        /// The texture whose image view is bound as the storage image.
        texture: &'a A::Texture,
    },
    /// A `SAMPLED_IMAGE` — a sampled image with the sampler bound separately.
    SampledImage {
        /// The texture whose image view is bound as the sampled image.
        texture: &'a A::Texture,
        /// The sampler bound (at this binding's separate sampler, backend-defined).
        sampler: &'a A::Sampler,
    },
    /// A `COMBINED_IMAGE_SAMPLER` — a sampled image bundled with its sampler.
    CombinedImage {
        /// The texture whose image view is bound as the sampled image.
        texture: &'a A::Texture,
        /// The sampler bound alongside the image (the COMBINED part).
        sampler: &'a A::Sampler,
    },
    /// A `STORAGE_BUFFER` — a read/write buffer.
    StorageBuffer {
        /// The buffer bound at this binding (its full range).
        buffer: &'a A::Buffer,
    },
    /// A `UNIFORM_BUFFER` — a read-only constant buffer.
    UniformBuffer {
        /// The buffer bound at this binding (its full range).
        buffer: &'a A::Buffer,
    },
}

/// Parameters for [`RhiDevice::create_bind_group`] (Render P1a).
///
/// Carries the [`RhiDevice::create_bind_group_layout`] layout the set is allocated
/// against plus one [`BindGroupEntry`] per layout entry, written into the layout's
/// bindings in slice order. `entries.len()` MUST equal the layout's entry count and
/// each entry's variant MUST match its layout entry's [`BindGroupLayoutEntry::kind`].
/// Both checks are REAL `debug_assert!`s in the backend: the layout retains a copy of
/// its per-entry `(binding, kind)` pairs, so `create_bind_group` cross-checks the
/// arity and the per-slot kind, and targets each write at the binding the layout
/// declared (positional slot `i` of the group ↔ slot `i` of the layout). Every
/// sampled image MUST be in
/// [`crate::enums::ImageLayout::ShaderReadOnlyOptimal`] / every storage image in
/// [`crate::enums::ImageLayout::General`] (transitioned via
/// [`crate::encoder::RhiCommandEncoder::image_barrier`]) before a draw/dispatch
/// accesses the bound set, or the validation layer faults at access time. The `'a`
/// lifetime borrows the layout + entries for the `create_bind_group` call only —
/// **but the resulting bind group retains each entry's resource handle in its
/// descriptor set.** CALLER CONTRACT: every retained resource MUST outlive every
/// submission that binds this group; dropping any before the binding submission
/// completes is use-after-free of a destroyed resource (caught by the validation
/// layer, not the Rust type system — the compile-time lifetime tie is deferred to
/// Phase 2-3, plan F1).
pub struct BindGroupDesc<'a, A: RhiApi> {
    /// The layout the descriptor set is allocated + written against.
    pub layout: &'a A::BindGroupLayout,
    /// One entry per binding, in binding order; its length and per-entry kind must
    /// match the layout's entries.
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

    /// Creates a bind-group layout (Render P1a: a `VkDescriptorSetLayout` from the
    /// desc's heterogeneous [`BindGroupLayoutEntry`] slice at `set 0`). Generalizes
    /// the prior COMBINED_IMAGE_SAMPLER-only form into the multi-resource descriptor
    /// vocabulary; supersedes the fixed compute descriptor layout for the graphics +
    /// vocabulary-compute paths.
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

    /// Creates a bind group (Render P1a: a `VkDescriptorPool` sized per the
    /// per-kind histogram + a single `VkDescriptorSet` allocated against
    /// `desc.layout` and written ONCE with one descriptor per `desc.entries` entry,
    /// each entry's variant matching its layout entry's [`DescriptorKind`]).
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
