//! The [`boyko_rhi`] trait implementation for the Vulkan backend — compute path
//! only (Phase 1, plan Waves C+D).
//!
//! [`Vulkan`] is the zero-sized [`RhiApi`] marker. [`VulkanContext`] implements
//! [`RhiDevice`]; a thin [`VulkanQueue`] implements [`RhiQueue`] (plan O1/Q2);
//! [`VulkanCommandEncoder`] implements [`RhiCommandEncoder`] (the hot recording
//! path). The fixed compute descriptor-set + pipeline layouts that once lived in
//! the dissolved `ComputeHarness` are now cached on the device ([`ComputeLayouts`],
//! plan Q1/W2), and the command pool + buffer + descriptor pool + set move onto
//! the encoder.
//!
//! # Ownership & teardown (preserved 1:1 from `ComputeHarness`)
//!
//! Every owned RHI resource ([`VulkanShaderModule`], [`ComputePipeline`],
//! [`VulkanFence`], [`VulkanCommandEncoder`]) is destroyed by value through the
//! matching `unsafe destroy_*` (plan D2): the move encodes "destroyed exactly
//! once" in the type system. The shared [`ComputeLayouts`] are torn down in
//! [`VulkanContext`]'s `Drop`, after the resources that reference them, mirroring
//! the original reverse-order discipline.
//!
//! # Single-thread / `!Send + !Sync`
//!
//! Per plan §5.3 the RHI boundary is touched only by the dispatcher in the
//! apply-window. [`VulkanQueue`] / [`VulkanCommandEncoder`] hold a raw
//! `*const DeviceFns` into the owning [`VulkanContext`] (which outlives them);
//! they are neither `Send` nor `Sync`, so the borrow can never cross a thread.
//!
//! # Seam associated-type bindings
//!
//! The deferred-seam types (`Surface`/`Swapchain`/`Texture`/…) are unbounded, so
//! any type satisfies them. The concrete on-screen `Surface<'ctx>` /
//! `Swapchain<'ctx>` / `Renderer<'ctx>` are lifetime-parameterized (not
//! `'static`) and stay used directly this phase (scope decision), so the seam
//! types bind to the cheapest sensible placeholder: `()` for the type-less ones
//! and the `'static` [`VkSemaphore`] for `Semaphore`.

use core::ffi::c_void;
use core::ptr::{self, NonNull};

use boyko_rhi::{
    BarrierDesc, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BufferBarrier, BufferCopy,
    BufferDesc, BufferImageCopy, ComputePipelineDesc, DescriptorKind, GraphicsPipelineDesc,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, IndexType, MemoryLocation, MipMode,
    RenderArea, RenderingDesc, RhiApi, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc,
    ShaderStage, TextureDesc, Viewport,
};

use crate::compute::ComputeError;
use crate::device::{DeviceFns, VulkanContext};
use crate::error::VulkanError;
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::texture::VulkanTexture;

/// The maximum number of color attachments a single `begin_rendering` scope
/// binds inline without heap allocation (Phase-6 S0). Sized for the basic-slice
/// deferred G-buffer (depth + albedo + normal + material ≈ 4) with headroom; rung
/// 1 binds exactly one. A `debug_assert!` traps an over-count.
const MAX_RENDERING_COLOR_ATTACHMENTS: usize = 8;

/// The maximum number of image→buffer copy regions recorded inline without heap
/// allocation. The basic-slice golden readback uses a single full-image region.
const MAX_IMAGE_COPY_REGIONS: usize = 4;

/// The maximum number of vertex attributes a rung-3 graphics pipeline declares
/// inline without heap allocation. Rung 3 uses two (position + color); the cap has
/// headroom for the basic-slice mesh vertex (position/normal/uv/color ≈ 4). A
/// `debug_assert!` traps an over-count at `create_graphics_pipeline`.
const MAX_VERTEX_ATTRIBUTES: usize = 8;

/// The maximum number of bindings a single bind group / its layout declares inline
/// without heap allocation (Phase-6 S0 rung 6). Sized for the lighting L0/L1 inputs
/// on a SINGLE resolve set with headroom, then raised to 16 for the SDF clip-map
/// (M4) per-level bindings (`N = brick::BRICK_LEVELS` levels × 2 resources on top of
/// the 0..=8 gbuffer bindings = 15 — see the agnostic
/// `boyko_rhi::MAX_BIND_GROUP_BINDINGS` docstring). SDFDDGI I(-1) raised it 16 → 19 to
/// reserve room for the 3 DDGI resolve bindings landed in rung I0 (this rung adds
/// none). A `debug_assert!` traps an over-count at
/// `create_bind_group_layout`/`create_bind_group`.
const MAX_BIND_GROUP_BINDINGS: usize = 19;

// The bind-group create path keeps its own copy of the cap so a future divergence
// from the agnostic `boyko_rhi::MAX_BIND_GROUP_BINDINGS` (the desc-side cap) breaks
// the build rather than silently truncating an over-count.
const _: () = assert!(
    MAX_BIND_GROUP_BINDINGS == boyko_rhi::MAX_BIND_GROUP_BINDINGS,
    "backend bind-group cap must match the agnostic boyko_rhi::MAX_BIND_GROUP_BINDINGS"
);

/// The five [`DescriptorKind`] slots, in a fixed order, used to bucket a bind
/// group's descriptors into a per-kind histogram for exact pool sizing (Render P1a).
/// [`DESCRIPTOR_KIND_VK`] maps each slot to its `VkDescriptorType`; the two arrays
/// share the slot order.
const DESCRIPTOR_KIND_VK: [i32; 5] = [
    VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
    VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE,
    VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
    VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
    VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
];

/// Maps a [`DescriptorKind`] to its histogram slot in [`DESCRIPTOR_KIND_VK`].
#[inline]
fn descriptor_kind_slot(kind: DescriptorKind) -> usize {
    match kind {
        DescriptorKind::CombinedImageSampler => 0,
        DescriptorKind::SampledImage => 1,
        DescriptorKind::StorageImage => 2,
        DescriptorKind::UniformBuffer => 3,
        DescriptorKind::StorageBuffer => 4,
    }
}

/// The [`DescriptorKind`] a [`BindGroupEntry`] variant carries (Render P1a). The
/// per-entry write's `descriptor_type` and the pool histogram both read this, so a
/// new variant must be handled here (exhaustive match, no wildcard).
#[inline]
fn bind_group_entry_kind(entry: &BindGroupEntry<Vulkan>) -> DescriptorKind {
    match entry {
        BindGroupEntry::StorageImage { .. } => DescriptorKind::StorageImage,
        BindGroupEntry::SampledImage { .. } => DescriptorKind::SampledImage,
        BindGroupEntry::CombinedImage { .. } => DescriptorKind::CombinedImageSampler,
        BindGroupEntry::StorageBuffer { .. } => DescriptorKind::StorageBuffer,
        BindGroupEntry::UniformBuffer { .. } => DescriptorKind::UniformBuffer,
    }
}

/// The cap on a graphics pipeline's color (MRT) attachment count, shared by the
/// pipeline's color-blend + dynamic-rendering format arrays (Phase-6 S0 rung 6). The
/// basic-slice G-buffer geometry pass uses two (albedo + normal); the cap matches the
/// `begin_rendering` color-attachment cap so a pipeline can target every attachment a
/// rendering scope binds. A `debug_assert!` traps an over-count.
const MAX_COLOR_ATTACHMENTS: usize = MAX_RENDERING_COLOR_ATTACHMENTS;

// The pipeline's MRT format/blend cap MUST NOT exceed the `begin_rendering`
// attachment cap, or a pipeline could declare more color targets than any rendering
// scope can bind — a guaranteed draw-time format/count mismatch (W2-b). Pinned at
// build time so a future cap edit cannot silently break the invariant.
const _: () = assert!(
    MAX_COLOR_ATTACHMENTS <= MAX_RENDERING_COLOR_ATTACHMENTS,
    "graphics-pipeline MRT cap must not exceed the begin_rendering attachment cap"
);

/// Byte size of the single COMPUTE push-constant range the device-shared
/// [`ComputeLayouts`]`::pipeline_layout` declares.
///
/// `ComputeLayouts` is **one shared layout** reused by every compute pipeline
/// (the `sdf_editlist` 4-byte path and the `sdf_depth_composite` marcher's
/// [`crate::compute::COMPOSITE_PUSH_CONSTANT_BYTES`]-byte path alike). A pipeline
/// layout may declare MORE push bytes than a given shader uses — that is valid
/// Vulkan; only declaring FEWER than a shader reads is the bug. So the shared
/// range is sized to the LARGEST consumer (the marcher) and every smaller-push
/// pipeline binds against it unchanged. Derived from the consumer constant, never
/// a magic literal, so a future widening of the marcher block re-sizes the range
/// automatically. The value stays within the Vulkan-guaranteed 128-byte floor for
/// `maxPushConstantsSize` (asserted below), so no device-limit query is required.
const COMPUTE_PUSH_CONSTANT_RANGE_BYTES: u32 = crate::compute::COMPOSITE_PUSH_CONSTANT_BYTES;

/// The Vulkan-guaranteed minimum `maxPushConstantsSize` (Vulkan 1.3 spec,
/// "Required Limits"). The shared compute push range must fit within it so the
/// layout is valid on every conformant device without probing the real limit.
const VULKAN_MIN_MAX_PUSH_CONSTANTS_SIZE: u32 = 128;

// The shared compute push range must be a non-empty multiple of 4 (Vulkan requires
// `size` be a multiple of 4 and `> 0`) and fit the guaranteed device floor. Pinned
// at build time so a future marcher-block edit cannot silently produce an invalid
// range or one that overflows the portable limit.
const _: () = assert!(
    COMPUTE_PUSH_CONSTANT_RANGE_BYTES > 0
        && COMPUTE_PUSH_CONSTANT_RANGE_BYTES.is_multiple_of(4)
        && COMPUTE_PUSH_CONSTANT_RANGE_BYTES <= VULKAN_MIN_MAX_PUSH_CONSTANTS_SIZE,
    "shared compute push range must be a non-empty multiple of 4 within the 128-byte floor"
);

/// A zeroed [`VkBufferImageCopy`] the inline-region array's unused tail slots
/// hold (never read: only the first `regions.len()` are passed to the driver).
const DEFAULT_BUFFER_IMAGE_COPY: VkBufferImageCopy = VkBufferImageCopy {
    buffer_offset: 0,
    buffer_row_length: 0,
    buffer_image_height: 0,
    image_subresource: VkImageSubresourceLayers {
        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    },
    image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
    image_extent: VkExtent3D {
        width: 0,
        height: 0,
        depth: 1,
    },
};

/// Lowers one agnostic [`BufferImageCopy`] to the FFI `VkBufferImageCopy`
/// (mapping the flattened scalar fields into the nested Vulkan structs). The
/// agnostic `aspect` bits equal the `VK_IMAGE_ASPECT_*` bits (identity cast,
/// asserted in `abi_guard.rs`).
#[inline]
fn vk_buffer_image_copy(r: &BufferImageCopy) -> VkBufferImageCopy {
    VkBufferImageCopy {
        buffer_offset: r.buffer_offset,
        buffer_row_length: r.buffer_row_length,
        buffer_image_height: r.buffer_image_height,
        image_subresource: VkImageSubresourceLayers {
            aspect_mask: r.aspect.bits(),
            mip_level: r.mip_level,
            base_array_layer: r.base_array_layer,
            layer_count: r.layer_count,
        },
        image_offset: VkOffset3D {
            x: r.image_offset_x,
            y: r.image_offset_y,
            z: r.image_offset_z,
        },
        image_extent: VkExtent3D {
            width: r.image_extent_w,
            height: r.image_extent_h,
            depth: r.image_extent_d,
        },
    }
}

/// The zero-sized Vulkan backend marker implementing [`RhiApi`] (plan D1).
///
/// Static dispatch only: every trait call monomorphizes to a direct
/// `(fns.cmd_*)` indirect call, byte-identical to the inherent FFI methods.
pub struct Vulkan;

impl RhiApi for Vulkan {
    type Device = VulkanContext;
    type Queue = VulkanQueue;
    type CommandEncoder = VulkanCommandEncoder;
    type Buffer = BoundBuffer;
    type ShaderModule = VulkanShaderModule;
    type ComputePipeline = ComputePipeline;
    type Fence = VulkanFence;

    // ===== DEFERRED SEAM — bound to the cheapest placeholder this phase. =====
    // The concrete on-screen `Surface`/`Swapchain`/`Renderer` are lifetime-bound
    // and used directly (scope decision); they cannot satisfy a `'static`
    // associated type, so `()` stands in until the Phase-2-3 on-screen-in-trait
    // surface lands. `Semaphore` binds to the concrete `VkSemaphore` (a `'static`
    // `u64` newtype) since that one fits. `Texture` binds to the S0
    // [`VulkanTexture`] now that `create_texture` is implemented.
    type Surface = ();
    type Swapchain = ();
    type Semaphore = VkSemaphore;
    type Texture = VulkanTexture;
    // `Sampler`/`BindGroupLayout`/`BindGroup` bind to the S0 rung-5 concrete types
    // now that `create_sampler`/`create_bind_group_layout`/`create_bind_group` are
    // implemented (the combined-image-sampler graphics descriptor surface).
    type Sampler = VulkanSampler;
    // `GraphicsPipeline` binds to the S0 rung-2 [`VulkanGraphicsPipeline`] now that
    // `create_graphics_pipeline` is implemented.
    type GraphicsPipeline = VulkanGraphicsPipeline;
    type BindGroup = VulkanBindGroup;
    type BindGroupLayout = VulkanBindGroupLayout;
}

/// The fixed Slice-0 compute layouts shared by every compute pipeline + command
/// encoder: one STORAGE_BUFFER @ set0/binding0 (COMPUTE) descriptor-set layout +
/// a pipeline layout with that set + a single COMPUTE push-constant range sized to
/// the largest consumer ([`COMPUTE_PUSH_CONSTANT_RANGE_BYTES`]).
///
/// Cached on [`VulkanContext`] (plan Q1/W2): created once on first
/// `create_compute_pipeline` / `create_command_encoder`, destroyed in the
/// context's `Drop` before `vkDestroyDevice`. The Phase-6 bind-group seam
/// supersedes this fixed layout.
pub struct ComputeLayouts {
    /// One STORAGE_BUFFER @ binding 0, COMPUTE stage.
    pub(crate) set_layout: VkDescriptorSetLayout,
    /// The set layout + a single COMPUTE push range of
    /// [`COMPUTE_PUSH_CONSTANT_RANGE_BYTES`] bytes (sized to the largest consumer).
    pub(crate) pipeline_layout: VkPipelineLayout,
}

impl ComputeLayouts {
    /// Builds the shared descriptor-set + pipeline layouts on `device`.
    ///
    /// On a partial failure the set layout is torn down before the error returns,
    /// so a failed `new` leaks nothing (the original `ComputeHarness::new`
    /// rollback, narrowed to just the layouts).
    pub(crate) fn new(device: VkDevice, fns: &DeviceFns) -> Result<Self, VulkanError> {
        let binding = VkDescriptorSetLayoutBinding {
            binding: 0,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        };
        let dsl_info = VkDescriptorSetLayoutCreateInfo {
            s_type: VkStructureType::DescriptorSetLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            binding_count: 1,
            p_bindings: &binding,
        };
        let mut set_layout = VkDescriptorSetLayout::NULL;
        // SAFETY: `device` is live; `dsl_info` is fully initialized and its
        // `p_bindings` points to the single `binding` local (alive for the call);
        // `&mut set_layout` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (fns.create_descriptor_set_layout)(device, &dsl_info, ptr::null(), &mut set_layout)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateDescriptorSetLayout", result));
        }

        let push_range = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            offset: 0,
            // Sized to the LARGEST compute consumer (the 80-byte `sdf_depth_composite`
            // marcher); the 4-byte `sdf_editlist` path binds against this wider range
            // unchanged — over-declaring push bytes is valid Vulkan.
            size: COMPUTE_PUSH_CONSTANT_RANGE_BYTES,
        };
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: VkStructureType::PipelineLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count: 1,
            p_set_layouts: &set_layout,
            push_constant_range_count: 1,
            p_push_constant_ranges: &push_range,
        };
        let mut pipeline_layout = VkPipelineLayout::NULL;
        // SAFETY: `device` is live; `pl_info` is fully initialized and references
        // the live `set_layout` + the `push_range` local; `&mut pipeline_layout`
        // is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (fns.create_pipeline_layout)(device, &pl_info, ptr::null(), &mut pipeline_layout)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `set_layout` was just created on `device` and is not yet
            // owned by any pipeline layout; destroy it exactly once on this error
            // path so it never leaks.
            unsafe { (fns.destroy_descriptor_set_layout)(device, set_layout, ptr::null()) };
            return Err(VulkanError::Vk("vkCreatePipelineLayout", result));
        }

        Ok(Self {
            set_layout,
            pipeline_layout,
        })
    }

    /// Destroys both layouts in reverse creation order, consuming `self`.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the layouts were created on; no
    /// compute pipeline or command encoder still referencing them is in flight;
    /// they are destroyed exactly once (the by-value `self` enforces the latter).
    pub(crate) unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
        // SAFETY: per the contract `device` is live and nothing references the
        // layouts; `vkDestroyPipelineLayout` then `vkDestroyDescriptorSetLayout`
        // are their matching teardown in reverse creation order, each once.
        unsafe {
            (fns.destroy_pipeline_layout)(device, self.pipeline_layout, ptr::null());
            (fns.destroy_descriptor_set_layout)(device, self.set_layout, ptr::null());
        }
    }
}

/// An owned compiled shader module ([`RhiApi::ShaderModule`]).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this module is
/// destroyed (via `destroy_shader_module`): the destroy goes through the context's
/// device fn-table. No compile-time `'ctx` tie this phase (plan F1; structural fix
/// deferred to Phase 2-3).
pub struct VulkanShaderModule {
    /// The `VkShaderModule` handle; destroyed by `destroy_shader_module`.
    pub(crate) module: VkShaderModule,
}

/// An owned compute pipeline ([`RhiApi::ComputePipeline`]).
///
/// Holds the `VkPipeline` + the `VkPipelineLayout` it was built against. Its shader
/// module is a separate owned [`VulkanShaderModule`] (the trait splits module +
/// pipeline creation). The layout is one of two (Render P1a):
///
/// * the device's shared [`ComputeLayouts`]`::pipeline_layout` (the fixed
///   single-STORAGE_BUFFER packed-buffer path) — then `owns_layout == false` and the
///   shared layout is NOT torn down with the pipeline; or
/// * a dedicated layout declaring `set 0` = a vocabulary bind-group layout + the
///   shared push range (`ComputePipelineDesc::bind_group_layout == Some`) — then
///   `owns_layout == true` and the layout is torn down with the pipeline (reverse
///   creation order: pipeline → layout) in `destroy_compute_pipeline`.
///
/// `layout` is the target a [`RhiCommandEncoder::bind_descriptor_set_compute`] binds
/// the vocabulary set against.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this pipeline is
/// destroyed (via `destroy_compute_pipeline`): the destroy goes through the
/// context's device fn-table, and the pipeline references the context's shared
/// layouts. No compile-time `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct ComputePipeline {
    /// The `VkPipeline` handle; destroyed first by `destroy_compute_pipeline`.
    pub(crate) pipeline: VkPipeline,
    /// The pipeline layout the pipeline was built against — either the device-shared
    /// `ComputeLayouts::pipeline_layout` (fixed path) or a dedicated layout (the
    /// vocabulary path). Read by `bind_descriptor_set_compute`.
    pub(crate) layout: VkPipelineLayout,
    /// `true` iff `layout` is a dedicated layout owned by this pipeline (destroyed
    /// after the pipeline); `false` for the device-shared layout (left alone).
    pub(crate) owns_layout: bool,
}

/// An owned graphics pipeline ([`RhiApi::GraphicsPipeline`], Phase-6 S0 rung 2).
///
/// Holds the `VkPipeline` **and** its own `VkPipelineLayout`. Unlike a compute
/// pipeline (which shares the device's [`ComputeLayouts`]`::pipeline_layout`), a
/// graphics pipeline uses a dedicated layout with no descriptor sets and either no
/// push range (rung 2) or one `VERTEX`-stage push range (rung 3's MVP `float4x4`),
/// created at `create_graphics_pipeline` and torn down with the pipeline (reverse
/// creation order: pipeline → layout) in `destroy_graphics_pipeline`. The layout is
/// also the target of [`RhiCommandEncoder::push_graphics_constants`]. Its shader
/// modules are separate caller-owned [`VulkanShaderModule`]s (the trait splits
/// module + pipeline creation).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this pipeline is
/// bound or destroyed: each goes through the context's device fn-table. No
/// compile-time `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct VulkanGraphicsPipeline {
    /// The `VkPipeline` handle; destroyed first by `destroy_graphics_pipeline`.
    pub(crate) pipeline: VkPipeline,
    /// The dedicated `VkPipelineLayout` (no descriptor sets; either no push range
    /// — rung 2 — or one VERTEX-stage push range — rung 3); destroyed after the pipeline.
    pub(crate) layout: VkPipelineLayout,
}

/// An owned texture sampler ([`RhiApi::Sampler`], Phase-6 S0 rung 5).
///
/// Holds the `VkSampler` handle; destroyed by value through `destroy_sampler`
/// (the move encodes "destroyed exactly once"). A sampler is device-global state
/// (no memory binding), so teardown is a single `vkDestroySampler`.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this sampler is used
/// (written into a bind group) or destroyed: each goes through the context's device
/// fn-table. No compile-time `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct VulkanSampler {
    /// The `VkSampler` handle; destroyed by `destroy_sampler`.
    pub(crate) sampler: VkSampler,
}

/// An owned bind-group (descriptor-set) layout ([`RhiApi::BindGroupLayout`],
/// Render P1a).
///
/// Holds the `VkDescriptorSetLayout` built from a slice of heterogeneous
/// [`BindGroupLayoutEntry`](boyko_rhi::BindGroupLayoutEntry)s at `set 0` (the
/// multi-resource descriptor vocabulary — combined-image-sampler, storage image,
/// storage buffer, and so on). Distinct from the device's shared compute
/// [`ComputeLayouts`]`::set_layout` (a runtime-mutable graphics + vocabulary-compute
/// layout, NOT the fixed compute one). Read at `create_bind_group` (set allocation)
/// and at `create_graphics_pipeline` / `create_compute_pipeline` (pipeline layout),
/// then destroyed by value through `destroy_bind_group_layout`.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this layout is used
/// or destroyed: each goes through the context's device fn-table. No compile-time
/// `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct VulkanBindGroupLayout {
    /// The `VkDescriptorSetLayout` (heterogeneous vocabulary bindings @ set 0).
    pub(crate) set_layout: VkDescriptorSetLayout,
    /// A fixed-capacity inline copy of the per-entry `(binding, kind)` pairs the
    /// layout was declared from (Render P1a, review M1/M2). POD, zero heap. Retained
    /// so `create_bind_group` can cross-check each [`BindGroupEntry`]'s variant
    /// against the declared [`DescriptorKind`] (M1) and target each write at the
    /// binding the layout actually declared rather than the slice index (M2). Only
    /// the first `entry_count` slots are valid; the tail is a harmless default.
    pub(crate) entries: [BindGroupLayoutBinding; MAX_BIND_GROUP_BINDINGS],
    /// The number of valid entries in `entries` (`1..=MAX_BIND_GROUP_BINDINGS`).
    pub(crate) entry_count: usize,
}

/// One layout entry's `(binding, kind)` pair, retained by [`VulkanBindGroupLayout`]
/// for the `create_bind_group` cross-check (Render P1a, review M1/M2). A trivial POD
/// (`Copy`, no heap), read only on the create/debug path — never on the per-frame
/// record path.
#[derive(Clone, Copy)]
pub(crate) struct BindGroupLayoutBinding {
    /// The binding index this entry was declared at (`layout(binding = N)`).
    pub(crate) binding: u32,
    /// The descriptor kind declared at this binding; cross-checked against the
    /// matching [`BindGroupEntry`]'s variant in `create_bind_group`.
    pub(crate) kind: DescriptorKind,
}

/// An owned bind group ([`RhiApi::BindGroup`], Render P1a).
///
/// Owns a dedicated `VkDescriptorPool` (sized per the layout's per-kind descriptor
/// histogram) plus the single `VkDescriptorSet` allocated + written ONCE from it.
/// Unlike the encoder's compute descriptor pool (one per encoder, fixed
/// STORAGE_BUFFER layout), a bind group is a standalone, caller-owned resource a
/// draw or a compute dispatch binds. The set is freed implicitly by destroying its
/// pool, so teardown is one `vkDestroyDescriptorPool` in `destroy_bind_group` (the
/// by-value move encodes "destroyed exactly once").
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this bind group is
/// bound or destroyed: each goes through the context's device fn-table. No
/// compile-time `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct VulkanBindGroup {
    /// The dedicated `VkDescriptorPool`; destroyed by `destroy_bind_group` (which
    /// also frees the set allocated from it).
    pub(crate) descriptor_pool: VkDescriptorPool,
    /// The single `VkDescriptorSet` allocated FROM `descriptor_pool` (read by
    /// `bind_descriptor_set`); freed implicitly when the pool is destroyed.
    pub(crate) descriptor_set: VkDescriptorSet,
}

/// An owned fence ([`RhiApi::Fence`]).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this fence is waited
/// on, reset, or destroyed: each goes through the context's device fn-table. No
/// compile-time `'ctx` tie this phase (plan F1; deferred to Phase 2-3).
pub struct VulkanFence {
    /// The `VkFence` handle; destroyed by `destroy_fence`.
    pub(crate) fence: VkFence,
}

/// A thin submission-queue wrapper ([`RhiQueue`], plan O1/Q2).
///
/// Holds the device's graphics+compute `VkQueue` + a raw `*const DeviceFns`
/// pointing into the owning context's **boxed** fn-table (plan A1). Because the
/// fn-table lives behind a stable heap address owned by the context, a context
/// move does not invalidate the pointer. `!Send + !Sync`: the pointer is
/// dereferenced only on the owning thread, and the context (hence the box)
/// outlives the queue wrapper; the RHI is single-threaded (§5.3).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive whenever this queue is
/// used (e.g. [`submit`](RhiQueue::submit)): submitting after the context is
/// dropped dangles the cached fn-table pointer = undefined behavior. There is no
/// compile-time `'ctx` tie this phase (the accepted plan-D2 trade-off; the
/// structural `'ctx` fix is deferred to Phase 2-3, plan F1).
pub struct VulkanQueue {
    queue: VkQueue,
    /// Raw pointer into the owning [`VulkanContext`]'s boxed [`DeviceFns`] — a
    /// stable heap address that survives context moves (plan A1) and outlives this
    /// wrapper (enforced by teardown order in the context's `Drop`).
    fns: *const DeviceFns,
}

/// The hot command-recording encoder ([`RhiCommandEncoder`]).
///
/// Owns its command pool + primary command buffer + descriptor pool + the one
/// fixed compute descriptor set (allocated ONCE here at
/// `create_command_encoder`, plan Q1 — no per-record `vkUpdateDescriptorSets`
/// regression). References the device's shared [`ComputeLayouts`] (cached copies
/// of the `Copy` layout handles) + a raw `*const DeviceFns` into the context's
/// **boxed** fn-table (stable across context moves, plan A1). `!Send + !Sync`.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive whenever this encoder is
/// used or destroyed: recording / destroying after the context is dropped dangles
/// the cached fn-table pointer = undefined behavior. There is no compile-time
/// `'ctx` tie this phase (the accepted plan-D2 trade-off; the structural `'ctx`
/// fix is deferred to Phase 2-3, plan F1).
pub struct VulkanCommandEncoder {
    /// Borrowed device handle (for `vkUpdateDescriptorSets`).
    device: VkDevice,
    /// Raw pointer into the owning [`VulkanContext`]'s boxed [`DeviceFns`] — a
    /// stable heap address that survives context moves (plan A1) and outlives this
    /// encoder (enforced by teardown order in the context's `Drop`).
    fns: *const DeviceFns,
    command_pool: VkCommandPool,
    /// Allocated FROM `command_pool`; freed implicitly when the pool is
    /// destroyed (a RESET_COMMAND_BUFFER pool, no explicit free needed).
    command_buffer: VkCommandBuffer,
    descriptor_pool: VkDescriptorPool,
    /// Allocated FROM `descriptor_pool`; freed implicitly with the pool.
    descriptor_set: VkDescriptorSet,
    /// Cached copy of the device's shared pipeline layout (for bind-set + push).
    pipeline_layout: VkPipelineLayout,
    /// The buffer the descriptor set currently points at — the set is updated
    /// only when a `bind_storage_buffer` names a different one (plan Q1).
    bound_buffer: VkBuffer,
    /// The descriptor-set index the next `dispatch` binds at (set by
    /// `bind_storage_buffer`; the Slice-0 contract is set 0).
    bound_set_index: u32,
}

// SAFETY: the encoder is a single-thread-only resource (plan §5.3): it is NOT
// `Send`/`Sync`. The `*const DeviceFns` points into the owning `VulkanContext`'s
// boxed fn-table — a stable heap address that a context move does not invalidate
// (plan A1) and that the context (hence the box) outlives, enforced by teardown
// order in the context's `Drop`; it is dereferenced only on the owning thread.
// The raw pointer makes the type `!Send + !Sync` by default (no auto-impl), which
// is the discipline we want — no explicit `unsafe impl` is added.

impl VulkanQueue {
    /// Wraps a context's queue + device fn-table into the thin RHI queue.
    ///
    /// `fns` must point to the owning context's [`DeviceFns`] (which outlives the
    /// returned queue).
    pub(crate) fn new(queue: VkQueue, fns: *const DeviceFns) -> Self {
        Self { queue, fns }
    }
}

impl VulkanContext {
    /// The thin RHI submission queue over this context's graphics+compute queue
    /// (plan O1/Q2). The returned [`VulkanQueue`] borrows this context's device
    /// fn-table and must not outlive the context.
    #[inline]
    pub fn rhi_queue(&self) -> VulkanQueue {
        VulkanQueue::new(self.queue(), self.device_fns() as *const DeviceFns)
    }
}

impl RhiDevice<Vulkan> for VulkanContext {
    type Error = VulkanError;

    fn create_buffer(&self, desc: &BufferDesc) -> Result<BoundBuffer, VulkanError> {
        // Plan A2: a zero-size buffer is an invalid request — it would yield a
        // `VkDescriptorBufferInfo.range == 0`. Reject it loud in debug; there is no
        // silent 0→1 size divergence (the created size, the stored
        // `BoundBuffer.size`, and the later descriptor `range` are all `desc.size`).
        debug_assert!(desc.size > 0, "invariant: zero-size buffer");
        // The agnostic `BufferUsage` bits equal the Vulkan `VK_BUFFER_USAGE_*`
        // bits (plan D5), so the projection is an identity cast on the u32 family.
        let usage: VkFlags = desc.usage.bits();
        match desc.location {
            // The host-visible foundation block (plan Q1).
            MemoryLocation::HostVisibleCoherent => {
                let block = self.host_block()?;
                let bound = block.borrow_mut().create_bound_buffer(desc.size, usage)?;
                Ok(bound)
            }
            // The Phase-5 device-local (VRAM) block (plan D3/MF-8). Always add the
            // `TRANSFER_SRC | TRANSFER_DST` usage so the staging upload + the
            // test-only readback (`vkCmdCopyBuffer`) can name the buffer as either
            // copy endpoint regardless of the caller's declared usage. The result
            // carries `mapped == None` — never host-mappable.
            MemoryLocation::DeviceLocal => {
                let usage = usage
                    | VK_BUFFER_USAGE_TRANSFER_SRC_BIT
                    | VK_BUFFER_USAGE_TRANSFER_DST_BIT;
                let block = self.device_block()?;
                let bound = block.borrow_mut().create_bound_buffer(desc.size, usage)?;
                Ok(bound)
            }
        }
    }

    unsafe fn destroy_buffer(&self, buffer: BoundBuffer) {
        // Plan A3: if a `BoundBuffer` exists, it was sub-allocated from one of the
        // shared blocks, so that block MUST already be initialized. A silent
        // early-return on `Err` here would drop the owned `buffer` WITHOUT
        // destroying its `VkBuffer` / returning its sub-allocation — a leak. The
        // matching block's `*_block()` only fails on the first-ever allocation
        // (which already happened to mint `buffer`), so these `expect`s are
        // unreachable by construction. `mapped` discriminates the origin block: a
        // host-visible buffer carries `Some(ptr)`, a device-local one `None`.
        if buffer.mapped.is_some() {
            let block = self
                .host_block()
                .expect("invariant: host block initialized when a host BoundBuffer exists");
            // SAFETY: `buffer` was produced by `create_buffer(HostVisibleCoherent)`
            // on this device's shared host block, the GPU is no longer using it
            // (caller fence-waited per the trait contract), and the by-value move
            // destroys it exactly once. The block is borrowed `&mut`
            // single-threaded.
            unsafe { block.borrow_mut().destroy_bound_buffer(buffer) };
        } else {
            let block = self
                .device_block()
                .expect("invariant: device block initialized when a device BoundBuffer exists");
            // SAFETY: `buffer` was produced by `create_buffer(DeviceLocal)` on this
            // device's shared device-local block, the GPU is no longer using it
            // (caller fence-waited), and the by-value move destroys it exactly once.
            // The block is borrowed `&mut` single-threaded.
            unsafe { block.borrow_mut().destroy_bound_buffer(buffer) };
        }
    }

    fn buffer_mapped_ptr(&self, buffer: &BoundBuffer) -> Option<NonNull<u8>> {
        // A host-visible buffer carries its persistent map pointer in `mapped`; a
        // device-local buffer carries `None` (it is never mapped, plan D3/MF-8),
        // honoring the device.rs:91 "`None` if not host-mappable" contract.
        buffer.mapped
    }

    fn create_texture(&self, desc: &TextureDesc) -> Result<VulkanTexture, VulkanError> {
        // SAFETY: `self.device()`/`self.device_fns()` are the live device + its
        // command table; `self.memory_properties()` are this physical device's
        // properties; `VulkanTexture::create` upholds the rest of the FFI
        // invariants internally (documented per `unsafe` block there).
        unsafe {
            VulkanTexture::create(
                self.device(),
                self.device_fns(),
                self.memory_properties(),
                desc,
            )
        }
    }

    unsafe fn destroy_texture(&self, texture: VulkanTexture) {
        // SAFETY: `texture` was created on this device by `create_texture`; the GPU
        // is no longer using it (caller fence-waited / `wait_idle`'d per the trait
        // contract); the by-value move destroys it exactly once. `destroy` tears
        // down the view → image → dedicated memory in reverse order.
        unsafe { texture.destroy(self.device(), self.device_fns()) };
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> Result<VulkanSampler, VulkanError> {
        // Rung 5 / GUI P5b: a deterministic sampler. The agnostic `Filter`/
        // `AddressMode` discriminants equal the `VkFilter`/`VkSamplerAddressMode`
        // constants (`as_i32()` no-op lowering, asserted in `abi_guard.rs`); the
        // single address mode applies to all three axes. Anisotropy / mip-bias /
        // compare are disabled (no `samplerAnisotropy` feature is requested at
        // device creation, so anisotropy MUST be FALSE).
        let address = desc.address_mode.as_i32();
        // GUI P5b Decision T4-D: map the agnostic `MipMode` to the Vulkan mip state.
        // `None` pins NEAREST mip mode + `minLod == maxLod == 0.0` (no mipmapping),
        // so a sampled read always reads the base level — the MSDF-atlas requirement
        // (a mipped read corrupts the per-channel median). It is the only variant in
        // P5b; the `match` makes the no-mip guarantee DECLARED, not accidental.
        let (mipmap_mode, min_lod, max_lod) = match desc.mip {
            MipMode::None => (VK_SAMPLER_MIPMAP_MODE_NEAREST, 0.0, 0.0),
        };
        // CSM Increment 0: lower the optional hardware depth-comparison op. `None`
        // keeps `compareEnable = VK_FALSE` + `compareOp = VK_COMPARE_OP_NEVER` —
        // byte-identical to every existing sampler. `Some(op)` builds a COMPARISON
        // sampler (`compareEnable = VK_TRUE`, `compareOp = op`) so a shadow-map PCF
        // read returns the filtered pass/fail of `reference (op) stored_depth`. The
        // agnostic `CompareOp` discriminant equals the `VkCompareOp` constant (asserted
        // in `abi_guard.rs`), so the lowering is an `as_i32()` no-op.
        let (compare_enable, compare_op) = match desc.compare {
            None => (VK_FALSE, VK_COMPARE_OP_NEVER),
            Some(op) => (VK_TRUE, op.as_i32()),
        };
        let info = VkSamplerCreateInfo {
            s_type: VkStructureType::SamplerCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            mag_filter: desc.mag_filter.as_i32(),
            min_filter: desc.min_filter.as_i32(),
            mipmap_mode,
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mip_lod_bias: 0.0,
            anisotropy_enable: VK_FALSE,
            max_anisotropy: 1.0,
            compare_enable,
            compare_op,
            min_lod,
            max_lod,
            border_color: VK_BORDER_COLOR_FLOAT_OPAQUE_BLACK,
            unnormalized_coordinates: VK_FALSE,
        };
        let mut sampler = VkSampler::NULL;
        // SAFETY: `device` is live; `info` is a fully-initialized `#[repr(C)]`
        // `VkSamplerCreateInfo` (null `p_next`, no GPU memory backing a sampler);
        // `&mut sampler` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_sampler)(self.device(), &info, ptr::null(), &mut sampler)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateSampler", result));
        }
        Ok(VulkanSampler { sampler })
    }

    unsafe fn destroy_sampler(&self, sampler: VulkanSampler) {
        // SAFETY: `sampler.sampler` was created on this device by `create_sampler`,
        // the GPU is no longer using it (caller fence-waited / `wait_idle`'d per the
        // trait contract), and the by-value move destroys it exactly once.
        unsafe {
            (self.device_fns().destroy_sampler)(self.device(), sampler.sampler, ptr::null())
        };
    }

    fn create_bind_group_layout(
        &self,
        desc: &BindGroupLayoutDesc,
    ) -> Result<VulkanBindGroupLayout, VulkanError> {
        // Render P1a: a heterogeneous set-0 layout from `desc.entries` — one
        // `VkDescriptorSetLayoutBinding` per entry, its `descriptor_type` the entry's
        // `DescriptorKind` cast `as i32` (the discriminants equal the
        // `VK_DESCRIPTOR_TYPE_*` constants — asserted in `abi_guard.rs`), its
        // `stage_flags` the entry's `ShaderStage` bits (identity cast, also asserted).
        // The bindings are a fixed-capacity inline array — zero heap allocation.
        let count = desc.entries.len();
        debug_assert!(
            (1..=MAX_BIND_GROUP_BINDINGS).contains(&count),
            "invariant: bind-group-layout entry count must be in 1..=MAX_BIND_GROUP_BINDINGS"
        );
        // Release-safe: clamp to the inline capacity (and a floor of 1) so the count
        // handed to the driver never exceeds the initialized slots even if a
        // (debug-asserted) out-of-range count slipped through a release build.
        let count = count.clamp(1, MAX_BIND_GROUP_BINDINGS);
        // Review M2: every declared binding must fit the inline-array capacity so the
        // retained `(binding, kind)` pairs (read at `create_bind_group` to target each
        // write) stay addressable. Debug-only; the contiguous-0..N convention every
        // call site uses trivially satisfies it.
        debug_assert!(
            desc.entries
                .iter()
                .take(count)
                .all(|e| (e.binding as usize) < MAX_BIND_GROUP_BINDINGS),
            "invariant: bind-group-layout binding must be < MAX_BIND_GROUP_BINDINGS"
        );
        // Retain the per-entry `(binding, kind)` pairs (review M1/M2) so
        // `create_bind_group` can cross-check the entry variant against the declared
        // kind and target each write at the layout's binding. POD copy, zero heap.
        let entries: [BindGroupLayoutBinding; MAX_BIND_GROUP_BINDINGS] =
            core::array::from_fn(|i| {
                if i < count {
                    BindGroupLayoutBinding {
                        binding: desc.entries[i].binding,
                        kind: desc.entries[i].kind,
                    }
                } else {
                    BindGroupLayoutBinding {
                        binding: i as u32,
                        kind: DescriptorKind::StorageBuffer,
                    }
                }
            });
        let bindings: [VkDescriptorSetLayoutBinding; MAX_BIND_GROUP_BINDINGS] =
            core::array::from_fn(|i| {
                if i < count {
                    let e = &desc.entries[i];
                    VkDescriptorSetLayoutBinding {
                        binding: e.binding,
                        descriptor_type: e.kind.as_i32(),
                        descriptor_count: e.count,
                        stage_flags: e.stage.bits(),
                        p_immutable_samplers: ptr::null(),
                    }
                } else {
                    VkDescriptorSetLayoutBinding {
                        binding: i as u32,
                        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                        descriptor_count: 0,
                        stage_flags: 0,
                        p_immutable_samplers: ptr::null(),
                    }
                }
            });
        let info = VkDescriptorSetLayoutCreateInfo {
            s_type: VkStructureType::DescriptorSetLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            binding_count: count as u32,
            p_bindings: bindings.as_ptr(),
        };
        let mut set_layout = VkDescriptorSetLayout::NULL;
        // SAFETY: `device` is live; `info` is fully initialized and its `p_bindings`
        // points to the first `count` (<= cap) entries of the live `bindings` inline
        // array (alive for the call), each a fully-initialized binding whose type +
        // stage come from `desc.entries[i]`; `&mut set_layout` is a valid out-pointer;
        // NULL allocator. `binding_count` bounds the driver's read to the initialized
        // prefix (the unused tail is never read).
        let raw = unsafe {
            (self.device_fns().create_descriptor_set_layout)(
                self.device(),
                &info,
                ptr::null(),
                &mut set_layout,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk(
                "vkCreateDescriptorSetLayout(bind group)",
                result,
            ));
        }
        Ok(VulkanBindGroupLayout {
            set_layout,
            entries,
            entry_count: count,
        })
    }

    unsafe fn destroy_bind_group_layout(&self, layout: VulkanBindGroupLayout) {
        // SAFETY: `layout.set_layout` was created on this device by
        // `create_bind_group_layout`, no bind group or pipeline referencing it is in
        // flight (caller contract), and the by-value move destroys it exactly once.
        unsafe {
            (self.device_fns().destroy_descriptor_set_layout)(
                self.device(),
                layout.set_layout,
                ptr::null(),
            )
        };
    }

    fn create_bind_group(
        &self,
        desc: &BindGroupDesc<Vulkan>,
    ) -> Result<VulkanBindGroup, VulkanError> {
        let device = self.device();
        let fns = self.device_fns();

        // Render P1a: one descriptor per `desc.entries` entry, written into the
        // layout's bindings in slice order. The count must equal the layout's entry
        // count and each entry's variant must match its layout entry's kind (caller
        // contract). The pool is sized per the per-kind histogram, the set is
        // allocated once, and `vkUpdateDescriptorSets` writes the whole set ONCE at
        // create — there is NO per-frame rewrite.
        let count = desc.entries.len();
        debug_assert!(
            (1..=MAX_BIND_GROUP_BINDINGS).contains(&count),
            "invariant: bind-group entry count must be in 1..=MAX_BIND_GROUP_BINDINGS"
        );
        // Review M1: the group's arity must equal the layout's declared entry count —
        // one descriptor write per layout binding, no more, no fewer. (The doc on
        // `BindGroupDesc` promises this check; it is now real because the layout
        // retains its `entry_count`.) Debug-only; vanishes in release.
        debug_assert!(
            count == desc.layout.entry_count,
            "P1a: BindGroupDesc.entries.len() must equal the layout's entry count"
        );
        let count = count.clamp(1, MAX_BIND_GROUP_BINDINGS);

        // --- Per-kind descriptor histogram → pool sizes (one entry per kind that
        //     actually appears, so the pool is sized exactly). The five kinds map onto
        //     fixed histogram slots; `pool_sizes` is a fixed inline array (zero heap). ---
        const KIND_COUNT: usize = 5;
        let mut hist = [0u32; KIND_COUNT];
        for entry in desc.entries.iter().take(count) {
            hist[descriptor_kind_slot(bind_group_entry_kind(entry))] += 1;
        }
        let mut pool_sizes = [VkDescriptorPoolSize {
            descriptor_type: 0,
            descriptor_count: 0,
        }; KIND_COUNT];
        let mut pool_size_count = 0usize;
        for (slot, &n) in hist.iter().enumerate() {
            if n > 0 {
                pool_sizes[pool_size_count] = VkDescriptorPoolSize {
                    descriptor_type: DESCRIPTOR_KIND_VK[slot],
                    descriptor_count: n,
                };
                pool_size_count += 1;
            }
        }
        let dp_info = VkDescriptorPoolCreateInfo {
            s_type: VkStructureType::DescriptorPoolCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            max_sets: 1,
            pool_size_count: pool_size_count as u32,
            p_pool_sizes: pool_sizes.as_ptr(),
        };
        let mut descriptor_pool = VkDescriptorPool::NULL;
        // SAFETY: `device` is live; `dp_info` is fully initialized referencing the
        // first `pool_size_count` (<= KIND_COUNT) entries of the live `pool_sizes`
        // inline array (alive for the call); `&mut descriptor_pool` is a valid
        // out-pointer; NULL allocator. `pool_size_count` bounds the driver's read.
        let raw = unsafe {
            (fns.create_descriptor_pool)(device, &dp_info, ptr::null(), &mut descriptor_pool)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateDescriptorPool(bind group)", result));
        }

        let set_layout = desc.layout.set_layout;
        let ds_alloc = VkDescriptorSetAllocateInfo {
            s_type: VkStructureType::DescriptorSetAllocateInfo,
            p_next: ptr::null(),
            descriptor_pool,
            descriptor_set_count: 1,
            p_set_layouts: &set_layout,
        };
        let mut descriptor_set = VkDescriptorSet::NULL;
        // SAFETY: `device` is live; `ds_alloc` names the live pool + references the
        // caller's live `set_layout` (the `set_layout` local, alive for the call);
        // `&mut descriptor_set` is a valid out-pointer.
        let raw =
            unsafe { (fns.allocate_descriptor_sets)(device, &ds_alloc, &mut descriptor_set) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `descriptor_pool` was just created and owns no live set yet
            // (allocation failed); destroy it once on this error path so it never
            // leaks (this also frees any partially-allocated set).
            unsafe { (fns.destroy_descriptor_pool)(device, descriptor_pool, ptr::null()) };
            return Err(VulkanError::Vk(
                "vkAllocateDescriptorSets(bind group)",
                result,
            ));
        }

        // --- Build the per-entry image-info + buffer-info inline arrays. Each kind
        //     populates exactly one of them at its slot; the WRITE at slot `i` points
        //     at whichever the kind reads (`p_image_info` for the three image kinds,
        //     `p_buffer_info` for the two buffer kinds), the other staying null. Each
        //     write's `dst_binding` is the LAYOUT entry's binding (caller contract:
        //     entries are in layout order, so `desc.layout`'s binding `i`). Image kinds
        //     declare the layout the descriptor records: GENERAL for a storage image,
        //     SHADER_READ_ONLY_OPTIMAL for a sampled one — the caller transitions each
        //     via `image_barrier` before access (the P1a SAFETY contract), and
        //     validation cross-checks the recorded layout at access time. All three
        //     inline arrays are fixed-capacity (zero heap) and outlive the update call. ---
        let mut image_infos = [VkDescriptorImageInfo {
            sampler: VkSampler::NULL,
            image_view: VkImageView::NULL,
            image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        }; MAX_BIND_GROUP_BINDINGS];
        let mut buffer_infos = [VkDescriptorBufferInfo {
            buffer: VkBuffer::NULL,
            offset: 0,
            range: 0,
        }; MAX_BIND_GROUP_BINDINGS];
        let writes: [VkWriteDescriptorSet; MAX_BIND_GROUP_BINDINGS] = core::array::from_fn(|i| {
            if i >= count {
                // Unused tail slot — never read (`descriptor_count: 0`, and the update
                // below passes only `count` writes). A harmless null-pointing write.
                return VkWriteDescriptorSet {
                    s_type: VkStructureType::WriteDescriptorSet,
                    p_next: ptr::null(),
                    dst_set: descriptor_set,
                    dst_binding: i as u32,
                    dst_array_element: 0,
                    descriptor_count: 0,
                    descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    p_image_info: ptr::null(),
                    p_buffer_info: ptr::null(),
                    p_texel_buffer_view: ptr::null(),
                };
            }
            let entry = &desc.entries[i];
            let kind = bind_group_entry_kind(entry);
            // Review M1: the entry's variant MUST match the kind the layout declared at
            // this slot (the doc-promised cross-check, now real because the layout
            // retains its per-entry kinds). The agnostic `BindGroupEntry` carries no
            // explicit binding; the layout↔group correspondence is positional, so slot
            // `i` of the group pairs with slot `i` of the layout. Debug-only.
            debug_assert!(
                kind == desc.layout.entries[i].kind,
                "P1a: BindGroupEntry variant must match the layout's DescriptorKind at this slot"
            );
            // Review M2: write at the binding the layout actually DECLARED, not the
            // positional slice index, so the write targets the right binding under any
            // binding numbering. For the contiguous-0..N convention every call site uses
            // (`layout.entries[i].binding == i`), this is byte-identical to the prior
            // positional `i as u32`.
            let dst_binding = desc.layout.entries[i].binding;
            let mut p_image_info: *const c_void = ptr::null();
            let mut p_buffer_info: *const VkDescriptorBufferInfo = ptr::null();
            match *entry {
                BindGroupEntry::StorageImage { texture } => {
                    image_infos[i] = VkDescriptorImageInfo {
                        sampler: VkSampler::NULL,
                        image_view: texture.view,
                        image_layout: VK_IMAGE_LAYOUT_GENERAL,
                    };
                    p_image_info = (&image_infos[i] as *const VkDescriptorImageInfo).cast();
                }
                BindGroupEntry::SampledImage { texture, sampler } => {
                    image_infos[i] = VkDescriptorImageInfo {
                        sampler: sampler.sampler,
                        image_view: texture.view,
                        image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    };
                    p_image_info = (&image_infos[i] as *const VkDescriptorImageInfo).cast();
                }
                BindGroupEntry::CombinedImage { texture, sampler } => {
                    // CSM Increment 1b: a MULTI-LAYER texture (array_view != NULL) binds its
                    // `VK_IMAGE_VIEW_TYPE_2D_ARRAY` sample view so a shader `Texture2DArray`
                    // resolves correctly (the cascade shadow map @ resolve binding 12). A
                    // single-layer texture has `array_view == NULL` → falls back to the
                    // full-subresource `.view`, BYTE-IDENTICAL to every existing combined-image
                    // caller (all bind single-layer images: present-blit, brick atlas, mesh-SDF).
                    let image_view = if texture.array_view != VkImageView::NULL {
                        texture.array_view
                    } else {
                        texture.view
                    };
                    image_infos[i] = VkDescriptorImageInfo {
                        sampler: sampler.sampler,
                        image_view,
                        image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    };
                    p_image_info = (&image_infos[i] as *const VkDescriptorImageInfo).cast();
                }
                BindGroupEntry::StorageBuffer { buffer }
                | BindGroupEntry::UniformBuffer { buffer } => {
                    buffer_infos[i] = VkDescriptorBufferInfo {
                        buffer: buffer.buffer,
                        offset: 0,
                        range: buffer.size,
                    };
                    p_buffer_info = &buffer_infos[i];
                }
            }
            VkWriteDescriptorSet {
                s_type: VkStructureType::WriteDescriptorSet,
                p_next: ptr::null(),
                dst_set: descriptor_set,
                dst_binding,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: kind.as_i32(),
                p_image_info,
                p_buffer_info,
                p_texel_buffer_view: ptr::null(),
            }
        });
        // SAFETY: `device` is live; the first `count` (<= cap) `writes` reference the
        // freshly-allocated `descriptor_set`, each at its layout entry's binding, with
        // `descriptor_type` matching that binding's kind. For an image kind
        // `p_image_info` points at the matching `image_infos[i]` local (which names the
        // caller's live image view + optional sampler); for a buffer kind
        // `p_buffer_info` points at `buffer_infos[i]` (which names the caller's live
        // buffer with its full range). The non-relevant pointer stays null, which the
        // driver ignores for that descriptor type. Both inline info arrays + the
        // `writes` array outlive the call; only the first `count` writes are passed
        // (the count bounds the driver's read). The set is not bound to any pending
        // command buffer (it was just allocated), so writing it is sound — and it is
        // written exactly ONCE here, never per-frame.
        unsafe {
            (fns.update_descriptor_sets)(device, count as u32, writes.as_ptr(), 0, ptr::null())
        };

        Ok(VulkanBindGroup {
            descriptor_pool,
            descriptor_set,
        })
    }

    unsafe fn destroy_bind_group(&self, group: VulkanBindGroup) {
        // SAFETY: `group.descriptor_pool` was created on this device by
        // `create_bind_group`, no submission using its set is pending (caller
        // fence-waited / `wait_idle`'d per the trait contract), and the by-value move
        // destroys it exactly once. Destroying the pool frees the set allocated from
        // it (no separate set free needed).
        unsafe {
            (self.device_fns().destroy_descriptor_pool)(
                self.device(),
                group.descriptor_pool,
                ptr::null(),
            )
        };
    }

    fn create_shader_module(&self, spirv: &[u32]) -> Result<VulkanShaderModule, VulkanError> {
        debug_assert!(!spirv.is_empty(), "invariant: SPIR-V word slice is non-empty");
        let sm_info = VkShaderModuleCreateInfo {
            s_type: VkStructureType::ShaderModuleCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            // `code_size` is in BYTES.
            code_size: spirv.len() * 4,
            p_code: spirv.as_ptr(),
        };
        let mut module = VkShaderModule::NULL;
        // SAFETY: `device` is live; `sm_info` is a fully-initialized `#[repr(C)]`
        // struct whose `p_code` points to `code_size` bytes of 4-byte-aligned
        // SPIR-V (`&[u32]` is word-aligned) alive for the call; `&mut module` is a
        // valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_shader_module)(
                self.device(),
                &sm_info,
                ptr::null(),
                &mut module,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateShaderModule", result));
        }
        Ok(VulkanShaderModule { module })
    }

    unsafe fn destroy_shader_module(&self, module: VulkanShaderModule) {
        // SAFETY: `module.module` was created on this device by
        // `create_shader_module`, no pipeline referencing it is in flight (caller
        // contract), and the by-value move destroys it exactly once.
        unsafe {
            (self.device_fns().destroy_shader_module)(self.device(), module.module, ptr::null())
        };
    }

    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<Vulkan>,
    ) -> Result<ComputePipeline, VulkanError> {
        // The device-shared compute pipeline layout declares one COMPUTE push range
        // of `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` (sized to the largest consumer). A
        // pipeline may USE fewer bytes than the layout declares (valid Vulkan), so any
        // request that is a non-empty multiple of 4 (Vulkan's push-range granularity)
        // and fits the shared range is accepted; a larger request would read past the
        // declared range, so it is rejected as `Unsupported`. This covers both the
        // 4-byte `sdf_editlist` path and the 80-byte `sdf_depth_composite` marcher.
        if desc.push_constant_bytes == 0
            || !desc.push_constant_bytes.is_multiple_of(4)
            || desc.push_constant_bytes > COMPUTE_PUSH_CONSTANT_RANGE_BYTES
        {
            return Err(VulkanError::Unsupported(
                "push_constant_bytes must be a multiple of 4 within the shared compute push range",
            ));
        }
        // The shared pipeline layout is needed at pipeline-create time (plan Q1).
        let layouts = self.compute_layouts()?;

        // Render P1a: pick the pipeline layout. `None` → the device-shared
        // single-STORAGE_BUFFER fixed layout (the packed-buffer path, byte-identical
        // to before, NOT owned by the pipeline). `Some(bgl)` → a DEDICATED layout
        // declaring `set 0` = the vocabulary bind-group layout + the shared COMPUTE
        // push range, owned by the pipeline and torn down with it. The dedicated
        // layout is created first; if pipeline creation fails below it is rolled back
        // before the error returns.
        let (pipeline_layout, owns_layout) = match desc.bind_group_layout {
            None => (layouts.pipeline_layout, false),
            Some(bgl) => {
                let set_layout = bgl.set_layout;
                let push_range = VkPushConstantRange {
                    stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                    offset: 0,
                    size: COMPUTE_PUSH_CONSTANT_RANGE_BYTES,
                };
                let pl_info = VkPipelineLayoutCreateInfo {
                    s_type: VkStructureType::PipelineLayoutCreateInfo,
                    p_next: ptr::null(),
                    flags: 0,
                    set_layout_count: 1,
                    p_set_layouts: &set_layout,
                    push_constant_range_count: 1,
                    p_push_constant_ranges: &push_range,
                };
                let mut layout = VkPipelineLayout::NULL;
                // SAFETY: `device` is live; `pl_info` is fully initialized referencing
                // the `set_layout` local (the caller's live vocabulary set-layout, alive
                // for this whole fn) at `set 0` + the `push_range` local (alive for this
                // whole fn); `&mut layout` is a valid out-pointer; NULL allocator.
                let raw = unsafe {
                    (self.device_fns().create_pipeline_layout)(
                        self.device(),
                        &pl_info,
                        ptr::null(),
                        &mut layout,
                    )
                };
                let result = VkResult::from_raw(raw);
                if !result.is_success() {
                    return Err(VulkanError::Vk("vkCreatePipelineLayout(compute)", result));
                }
                (layout, true)
            }
        };

        let stage = VkPipelineShaderStageCreateInfo {
            s_type: VkStructureType::PipelineShaderStageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: desc.module.module,
            p_name: desc.entry.as_ptr(),
            p_specialization_info: ptr::null(),
        };
        let cp_info = VkComputePipelineCreateInfo {
            s_type: VkStructureType::ComputePipelineCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage,
            layout: pipeline_layout,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };
        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `device` is live; null pipeline cache (`0`) is valid; one
        // create-info is fully initialized, referencing the live shader module +
        // the chosen `pipeline_layout` (the device-shared fixed layout or the
        // just-created dedicated one); `&mut pipeline` is a valid out-pointer for the
        // single pipeline; NULL allocator. The module is owned by the caller's
        // `VulkanShaderModule`, alive for this call.
        let raw = unsafe {
            (self.device_fns().create_compute_pipelines)(
                self.device(),
                0,
                1,
                &cp_info,
                ptr::null(),
                &mut pipeline,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            if owns_layout {
                // SAFETY: the dedicated `pipeline_layout` was just created on this
                // device and is not yet owned by any pipeline (creation failed);
                // destroy it once on this error path so it never leaks. The shared
                // layout (`owns_layout == false`) is left alone — it is the device's.
                unsafe {
                    (self.device_fns().destroy_pipeline_layout)(
                        self.device(),
                        pipeline_layout,
                        ptr::null(),
                    )
                };
            }
            return Err(VulkanError::from(ComputeError::VkError(
                "vkCreateComputePipelines",
                result,
            )));
        }
        Ok(ComputePipeline {
            pipeline,
            layout: pipeline_layout,
            owns_layout,
        })
    }

    unsafe fn destroy_compute_pipeline(&self, pipeline: ComputePipeline) {
        // SAFETY: `pipeline.pipeline` was created on this device, no submission
        // using it is pending (caller contract), and the by-value move destroys it
        // exactly once. A dedicated layout (`owns_layout`) is torn down AFTER the
        // pipeline (reverse creation order); the device-shared layout is left alone.
        unsafe {
            (self.device_fns().destroy_pipeline)(self.device(), pipeline.pipeline, ptr::null());
            if pipeline.owns_layout {
                (self.device_fns().destroy_pipeline_layout)(
                    self.device(),
                    pipeline.layout,
                    ptr::null(),
                );
            }
        };
    }

    fn create_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        let device = self.device();
        let fns = self.device_fns();

        // --- The pipeline layout. Rungs 2..4 use a layout with NO descriptor sets
        //     (rung 2 empty; rung 3/4 add ONE `VERTEX`-stage push-constant range of
        //     `desc.push_constant_bytes` bytes at offset 0 — the MVP `float4x4`).
        //     Rung 5 ADDS one bind-group layout at `set 0` (the COMBINED_IMAGE_SAMPLER)
        //     when `desc.bind_group_layout` is `Some`, so a `bind_descriptor_set` can
        //     bind a matching group before the sampling draw; `None` keeps the
        //     rungs-2..4 no-descriptor path byte-identical (count 0, null array).
        //     Created first; if pipeline creation fails below, it is torn down before
        //     the error returns (reverse-order rollback). The `push_range` +
        //     `set_layout` locals must outlive the create call, so they are bound
        //     here (the layout-info pointers below reference them). ---
        // The push range spans `VERTEX | FRAGMENT`: every existing graphics shader pushes from the
        // VERTEX stage only (the gbuffer/cascade/spot pipelines), and a fragment stage that declares
        // no push block simply ignores the range — so widening the visibility is byte-neutral for
        // them. The Shadow Phase 5 Inc-2 POINT depth FS (`punctual_depth.fs`) READS the `cam_eye@64`
        // lane (`light_pos`/`inv_range`), which requires the range to cover `FRAGMENT`. Push-constant
        // stage flags are part of the pipeline LAYOUT, not the recorded command stream, and the
        // recorders keep pushing with `VK_SHADER_STAGE_VERTEX_BIT` (a subset), so the rendered output
        // of every pre-Inc-2 pipeline is unchanged (the 0%-gate holds).
        let push_range = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
            offset: 0,
            size: desc.push_constant_bytes,
        };
        let has_push = desc.push_constant_bytes > 0;
        let set_layout = desc
            .bind_group_layout
            .map_or(VkDescriptorSetLayout::NULL, |bgl| bgl.set_layout);
        let has_set = desc.bind_group_layout.is_some();
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: VkStructureType::PipelineLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count: u32::from(has_set),
            p_set_layouts: if has_set { &set_layout } else { ptr::null() },
            push_constant_range_count: u32::from(has_push),
            p_push_constant_ranges: if has_push {
                &push_range
            } else {
                ptr::null()
            },
        };
        let mut layout = VkPipelineLayout::NULL;
        // SAFETY: `device` is live; `pl_info` is fully initialized with either zero
        // descriptor sets (null array valid for count 0) or one set pointing at the
        // `set_layout` local (the caller's live bind-group set-layout, alive for this
        // whole fn) when `has_set`, and either zero push ranges (null array valid for
        // count 0) or one range pointing at the `push_range` local (alive for this
        // whole fn) when `has_push`; `&mut layout` is a valid out-pointer; NULL
        // allocator.
        let raw =
            unsafe { (fns.create_pipeline_layout)(device, &pl_info, ptr::null(), &mut layout) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreatePipelineLayout(graphics)", result));
        }

        // --- Two shader stages (vertex + fragment). ---
        let stages = [
            VkPipelineShaderStageCreateInfo {
                s_type: VkStructureType::PipelineShaderStageCreateInfo,
                p_next: ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_VERTEX_BIT,
                module: desc.vertex_module.module,
                p_name: desc.vertex_entry.as_ptr(),
                p_specialization_info: ptr::null(),
            },
            VkPipelineShaderStageCreateInfo {
                s_type: VkStructureType::PipelineShaderStageCreateInfo,
                p_next: ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_FRAGMENT_BIT,
                module: desc.fragment_module.module,
                p_name: desc.fragment_entry.as_ptr(),
                p_specialization_info: ptr::null(),
            },
        ];

        // --- Vertex input. Rung 2: empty (positions come from the vertex shader's
        //     SV_VertexID — no vertex buffer). Rung 3: one binding (binding 0,
        //     per-vertex rate, the layout's stride) + one attribute per layout entry.
        //     The `binding`/`attributes` locals must outlive the create call below;
        //     they are bound here so the `vertex_input` pointers stay valid. The
        //     unused tail of `attributes` (slots >= the layout's count) is never read:
        //     `vertex_attribute_description_count` bounds the driver's read. ---
        let mut vk_bindings: [VkVertexInputBindingDescription; 1] =
            [VkVertexInputBindingDescription {
                binding: 0,
                stride: 0,
                input_rate: VK_VERTEX_INPUT_RATE_VERTEX,
            }];
        let mut vk_attributes: [VkVertexInputAttributeDescription; MAX_VERTEX_ATTRIBUTES] =
            core::array::from_fn(|_| VkVertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: VK_FORMAT_UNDEFINED,
                offset: 0,
            });
        let attribute_count = match &desc.vertex_layout {
            None => 0usize,
            Some(layout) => {
                debug_assert!(
                    layout.attributes.len() <= MAX_VERTEX_ATTRIBUTES,
                    "invariant: rung-3 vertex layout has <= MAX_VERTEX_ATTRIBUTES attributes"
                );
                vk_bindings[0].stride = layout.stride;
                // The agnostic `VertexFormat` discriminant equals the `VkFormat`
                // constant (asserted in `abi_guard.rs`).
                for (slot, attr) in vk_attributes.iter_mut().zip(layout.attributes.iter()) {
                    slot.location = attr.location;
                    slot.binding = 0;
                    slot.format = attr.format.as_i32();
                    slot.offset = attr.offset;
                }
                // Release-safe: the count handed to the driver never exceeds the
                // initialized inline slots, even if a (debug-asserted above) over-count
                // were to slip through in a release build — `vertex_attribute_description_count`
                // then matches exactly the slots written by the `zip` loop.
                layout.attributes.len().min(MAX_VERTEX_ATTRIBUTES)
            }
        };
        let has_vertex_layout = attribute_count > 0;
        let vertex_input = VkPipelineVertexInputStateCreateInfo {
            s_type: VkStructureType::PipelineVertexInputStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            vertex_binding_description_count: u32::from(has_vertex_layout),
            p_vertex_binding_descriptions: if has_vertex_layout {
                vk_bindings.as_ptr()
            } else {
                ptr::null()
            },
            vertex_attribute_description_count: attribute_count as u32,
            p_vertex_attribute_descriptions: if has_vertex_layout {
                vk_attributes.as_ptr()
            } else {
                ptr::null()
            },
        };

        let input_assembly = VkPipelineInputAssemblyStateCreateInfo {
            s_type: VkStructureType::PipelineInputAssemblyStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            // The agnostic `PrimitiveTopology` discriminant equals the
            // `VkPrimitiveTopology` constant (asserted in `abi_guard.rs`).
            topology: desc.topology.as_i32(),
            primitive_restart_enable: VK_FALSE,
        };

        // Dynamic viewport + scissor: counts of 1 with null pointers (the rects come
        // from `cmd_set_viewport`/`cmd_set_scissor`, recorded before the draw).
        let viewport_state = VkPipelineViewportStateCreateInfo {
            s_type: VkStructureType::PipelineViewportStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            viewport_count: 1,
            p_viewports: ptr::null(),
            scissor_count: 1,
            p_scissors: ptr::null(),
        };

        // CSM Increment 0: lower the configurable cull mode + optional depth bias.
        // `CullMode::None` == `VK_CULL_MODE_NONE` and `depth_bias: None` ==
        // `depthBiasEnable = VK_FALSE` + zeroed factors — byte-identical to the prior
        // hardcoded rasterization state, so every existing pipeline (which passes those
        // defaults) re-emits the SAME bytes. The agnostic `CullMode` discriminant
        // equals the `VkCullModeFlags` bits (asserted in `abi_guard.rs`), so the cull
        // lowering is an `as_u32()` no-op. A shadow-map depth pass selects
        // `CullMode::Front` + `Some(DepthBias { .. })`.
        let cull_mode: VkFlags = desc.cull_mode.as_u32();
        let (depth_bias_enable, db_constant, db_slope, db_clamp) = match desc.depth_bias {
            None => (VK_FALSE, 0.0, 0.0, 0.0),
            Some(b) => (VK_TRUE, b.constant_factor, b.slope_factor, b.clamp),
        };
        let rasterization = VkPipelineRasterizationStateCreateInfo {
            s_type: VkStructureType::PipelineRasterizationStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            depth_clamp_enable: VK_FALSE,
            rasterizer_discard_enable: VK_FALSE,
            polygon_mode: VK_POLYGON_MODE_FILL,
            cull_mode,
            front_face: VK_FRONT_FACE_COUNTER_CLOCKWISE,
            depth_bias_enable,
            depth_bias_constant_factor: db_constant,
            depth_bias_clamp: db_clamp,
            depth_bias_slope_factor: db_slope,
            line_width: 1.0,
        };

        let multisample = VkPipelineMultisampleStateCreateInfo {
            s_type: VkStructureType::PipelineMultisampleStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            rasterization_samples: VK_SAMPLE_COUNT_1_BIT,
            sample_shading_enable: VK_FALSE,
            min_sample_shading: 0.0,
            p_sample_mask: ptr::null(),
            alpha_to_coverage_enable: VK_FALSE,
            alpha_to_one_enable: VK_FALSE,
        };

        // One opaque (blend-disabled) color-blend attachment state PER color (MRT)
        // attachment, each with an all-channel write mask so the fragment color reaches
        // every channel of its target (Phase-6 S0 rung 6). The G-buffer geometry pass
        // declares two (albedo + normal); rungs 2..5 declare one (`color_formats.len()
        // == 1`). The count MUST equal the dynamic-rendering format count below, or the
        // driver rejects the pipeline. The first `color_attachment_count` entries are
        // identical opaque states; the inline tail is never read (the count bounds it).
        //
        // CSM Increment 0: an EMPTY `color_formats` is the DEPTH-ONLY path —
        // `colorAttachmentCount = 0`, a null color-blend attachment array, and a null
        // `pColorAttachmentFormats` below (a depth-only shadow-map pass). A depth-only
        // pipeline then REQUIRES a depth format (validation rejects a pipeline with
        // neither color nor depth); the relaxed assert pins that.
        debug_assert!(
            !desc.color_formats.is_empty() || desc.depth_format.is_some(),
            "invariant: a graphics pipeline needs >= 1 color attachment format OR a depth format (depth-only)"
        );
        debug_assert!(
            desc.color_formats.len() <= MAX_COLOR_ATTACHMENTS,
            "invariant: graphics pipeline color-attachment count exceeds the fixed cap"
        );
        // Release-safe: the count handed to the driver never exceeds the initialized
        // inline slots even if a (debug-asserted) over-count slipped through a release
        // build — it is clamped to the cap, matching the arrays' length.
        let color_attachment_count = desc.color_formats.len().min(MAX_COLOR_ATTACHMENTS);
        // GUI P5a Decision 3: lower the optional `BlendState`. `None` keeps the
        // pre-P5a opaque (blend-disabled) write byte-identical (every existing
        // pipeline). `Some(bs)` enables blending with `bs`'s factors/op on ALL color
        // attachments (P5a UI is single-target; a future MRT-per-target widening
        // turns `Option<BlendState>` into a slice). The agnostic `BlendFactor`/
        // `BlendOp` discriminants equal the `VkBlendFactor`/`VkBlendOp` constants
        // (asserted in `abi_guard.rs`), so each lowering is an `as_i32()` no-op.
        let (blend_enable, src_color, dst_color, color_op, src_alpha, dst_alpha, alpha_op) =
            match desc.blend {
                None => (VK_FALSE, 0, 0, 0, 0, 0, 0),
                Some(bs) => (
                    VK_TRUE,
                    bs.src_color.as_i32(),
                    bs.dst_color.as_i32(),
                    bs.color_op.as_i32(),
                    bs.src_alpha.as_i32(),
                    bs.dst_alpha.as_i32(),
                    bs.alpha_op.as_i32(),
                ),
            };
        // `from_fn` (not `[x; N]`) avoids requiring `Copy` on the FFI struct; every
        // slot is the identical (opaque or blended) all-channel-write state.
        let blend_attachments: [VkPipelineColorBlendAttachmentState; MAX_COLOR_ATTACHMENTS] =
            core::array::from_fn(|_| VkPipelineColorBlendAttachmentState {
                blend_enable,
                src_color_blend_factor: src_color,
                dst_color_blend_factor: dst_color,
                color_blend_op: color_op,
                src_alpha_blend_factor: src_alpha,
                dst_alpha_blend_factor: dst_alpha,
                alpha_blend_op: alpha_op,
                color_write_mask: VK_COLOR_COMPONENT_R_BIT
                    | VK_COLOR_COMPONENT_G_BIT
                    | VK_COLOR_COMPONENT_B_BIT
                    | VK_COLOR_COMPONENT_A_BIT,
            });
        // CSM Increment 0: a non-empty `color_formats` keeps the prior
        // `p_attachments = blend_attachments.as_ptr()` (byte-identical). An EMPTY
        // (depth-only) pipeline has `attachment_count = 0` + a null `p_attachments`,
        // and the WHOLE color-blend state is omitted (`p_color_blend_state = null`
        // below) — Vulkan allows a null color-blend state when there are no color
        // attachments.
        let has_color = color_attachment_count > 0;
        let color_blend = VkPipelineColorBlendStateCreateInfo {
            s_type: VkStructureType::PipelineColorBlendStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            logic_op_enable: VK_FALSE,
            logic_op: 0,
            attachment_count: color_attachment_count as u32,
            p_attachments: if has_color {
                blend_attachments.as_ptr()
            } else {
                ptr::null()
            },
            blend_constants: [0.0; 4],
        };
        let p_color_blend_state: *const VkPipelineColorBlendStateCreateInfo = if has_color {
            &color_blend
        } else {
            ptr::null()
        };

        let dynamic_states = [VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR];
        let dynamic_state = VkPipelineDynamicStateCreateInfo {
            s_type: VkStructureType::PipelineDynamicStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            dynamic_state_count: dynamic_states.len() as u32,
            p_dynamic_states: dynamic_states.as_ptr(),
        };

        // Depth-stencil state (Phase-6 S0 rung 4). Declared ONLY when a depth format
        // is present: depth test + write enabled, compare op LESS (nearer fragment
        // wins), no depth-bounds, no stencil. A `None` `depth_format` (rungs 1..3)
        // leaves both the depth-stencil pointer null and `depth_attachment_format`
        // UNDEFINED, so the rung-2/3 no-depth pipelines stay byte-identical. The
        // `depth_state` local must outlive the create call, so it is bound here. The
        // agnostic `Format` discriminant equals the `VkFormat` constant (asserted in
        // `abi_guard.rs`); `VK_COMPARE_OP_LESS` is the FFI constant.
        let depth_state = VkPipelineDepthStencilStateCreateInfo {
            s_type: VkStructureType::PipelineDepthStencilStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            depth_test_enable: VK_TRUE,
            depth_write_enable: VK_TRUE,
            depth_compare_op: VK_COMPARE_OP_LESS,
            depth_bounds_test_enable: VK_FALSE,
            stencil_test_enable: VK_FALSE,
            front: VkStencilOpState::default(),
            back: VkStencilOpState::default(),
            min_depth_bounds: 0.0,
            max_depth_bounds: 1.0,
        };
        let depth_attachment_format = match desc.depth_format {
            Some(fmt) => fmt.as_i32(),
            None => VK_FORMAT_UNDEFINED,
        };
        let p_depth_stencil_state: *const c_void = if desc.depth_format.is_some() {
            (&depth_state as *const VkPipelineDepthStencilStateCreateInfo).cast()
        } else {
            ptr::null()
        };

        // The dynamic-rendering attachment-format chain (no `VkRenderPass`). The
        // color-attachment formats declared here are the W2-b SAFETY contract: each
        // MUST equal the format of the same-index `begin_rendering` color attachment
        // any bound pipeline renders into — AND the count must equal the rendering
        // scope's color-attachment count — or the validation layer faults at DRAW
        // time. The format count equals `color_attachment_count` (the same value the
        // color-blend `attachment_count` above uses, so the two stay consistent).
        // `depth_attachment_format` carries the same contract for the depth attachment
        // (rung 4). The agnostic `Format` discriminant equals the `VkFormat` constant
        // (asserted in `abi_guard.rs`). The `color_formats` inline array's first
        // `color_attachment_count` entries are lowered from the desc; the tail is never
        // read (the count bounds the driver's read).
        let mut color_formats: [i32; MAX_COLOR_ATTACHMENTS] = [VK_FORMAT_UNDEFINED; MAX_COLOR_ATTACHMENTS];
        for (slot, fmt) in color_formats.iter_mut().zip(desc.color_formats.iter()) {
            *slot = fmt.as_i32();
        }
        let rendering_info = VkPipelineRenderingCreateInfo {
            s_type: VkStructureType::PipelineRenderingCreateInfo,
            p_next: ptr::null(),
            view_mask: 0,
            color_attachment_count: color_attachment_count as u32,
            // CSM Increment 0: null the format array for the depth-only path
            // (`color_attachment_count == 0`); the non-empty path is byte-identical.
            p_color_attachment_formats: if has_color {
                color_formats.as_ptr()
            } else {
                ptr::null()
            },
            depth_attachment_format,
            stencil_attachment_format: VK_FORMAT_UNDEFINED,
        };

        let gp_info = VkGraphicsPipelineCreateInfo {
            s_type: VkStructureType::GraphicsPipelineCreateInfo,
            // Chain the dynamic-rendering format struct (no render pass, OQ-6).
            p_next: (&rendering_info as *const VkPipelineRenderingCreateInfo).cast(),
            flags: 0,
            stage_count: stages.len() as u32,
            p_stages: stages.as_ptr(),
            p_vertex_input_state: &vertex_input,
            p_input_assembly_state: &input_assembly,
            p_tessellation_state: ptr::null(),
            p_viewport_state: &viewport_state,
            p_rasterization_state: &rasterization,
            p_multisample_state: &multisample,
            p_depth_stencil_state,
            p_color_blend_state,
            p_dynamic_state: &dynamic_state,
            layout,
            // Dynamic rendering: no render pass object (OQ-6, CLOSED).
            render_pass: 0,
            subpass: 0,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };

        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `device` is live; null pipeline cache (`0`) is valid; one
        // fully-initialized `VkGraphicsPipelineCreateInfo` references the live
        // `layout` (which references the `push_range` local for `has_push`, alive for
        // this whole fn), the two live caller-owned shader modules (via `stages`,
        // alive for the call), and the complete set of fixed-function sub-state
        // structs + dynamic-rendering format chain (all stack locals alive for the
        // call). `vertex_input` points at the `vk_bindings`/`vk_attributes` locals
        // when `has_vertex_layout`, whose first `attribute_count` (<= cap, asserted)
        // entries are initialized and bound by the driver's matching counts; an empty
        // layout uses null arrays with count 0. Tessellation state is null; the
        // depth-stencil state is the `depth_state` local (alive for this whole fn)
        // when `desc.depth_format` is `Some`, else null (rungs 1..3). `render_pass`
        // is `VK_NULL_HANDLE` (dynamic rendering). `&mut pipeline` is a valid
        // out-pointer for the single pipeline; NULL allocator.
        //
        // `p_color_blend_state` points at the `color_blend` local whose
        // `p_attachments` is the `blend_attachments` inline array (alive for the call),
        // its first `color_attachment_count` entries (= the format count below) read by
        // the driver — OR is null for the DEPTH-ONLY path (`color_attachment_count == 0`),
        // valid because Vulkan permits a null color-blend state with no color
        // attachments. The dynamic-rendering format chain's `p_color_attachment_formats`
        // is the `color_formats` inline array (alive for the call), first
        // `color_attachment_count` entries valid — OR null for the depth-only path.
        //
        // FORMAT CONTRACT (W2-b): `rendering_info.p_color_attachment_formats` declares
        // `desc.color_formats` (count + per-index format) and `.depth_attachment_format`
        // declares the rung-4 depth format; each MUST equal the same-index format (and
        // the count) of every `begin_rendering` color/depth attachment the pipeline is
        // later bound inside, or validation faults at draw time (not here). The MRT
        // color-blend `attachment_count` equals the format count, so the two never
        // disagree. The agnostic↔Vk discriminant equality is asserted in
        // `abi_guard.rs`; the cross-check against the bound rendering scope is the
        // caller's contract (encoded in `GraphicsPipelineDesc`↔`RenderingDesc`).
        let raw = unsafe {
            (fns.create_graphics_pipelines)(device, 0, 1, &gp_info, ptr::null(), &mut pipeline)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: the `layout` was created above and is not yet owned by any
            // pipeline (creation failed); destroy it exactly once on this error path
            // so it never leaks. NOTE: this single-handle rollback is correct ONLY
            // because `create_info_count == 1` above; a future BATCHED create path must
            // additionally destroy the successfully-created pipelines that
            // `vkCreateGraphicsPipelines` writes alongside VK_NULL_HANDLE on partial
            // failure (per-handle cleanup), or they leak.
            unsafe { (fns.destroy_pipeline_layout)(device, layout, ptr::null()) };
            return Err(VulkanError::Vk("vkCreateGraphicsPipelines", result));
        }

        Ok(VulkanGraphicsPipeline { pipeline, layout })
    }

    unsafe fn destroy_graphics_pipeline(&self, pipeline: VulkanGraphicsPipeline) {
        // SAFETY: both handles were created on this device by
        // `create_graphics_pipeline`, no submission using the pipeline is pending
        // (caller contract), and the by-value move destroys each exactly once.
        // Reverse creation order: the pipeline (created last) is destroyed before its
        // dedicated empty layout (created first).
        unsafe {
            (self.device_fns().destroy_pipeline)(self.device(), pipeline.pipeline, ptr::null());
            (self.device_fns().destroy_pipeline_layout)(
                self.device(),
                pipeline.layout,
                ptr::null(),
            );
        }
    }

    fn create_fence(&self, signaled: bool) -> Result<VulkanFence, VulkanError> {
        let fence_info = VkFenceCreateInfo {
            s_type: VkStructureType::FenceCreateInfo,
            p_next: ptr::null(),
            // `VK_FENCE_CREATE_SIGNALED_BIT` == 0x1.
            flags: if signaled { 0x0000_0001 } else { 0 },
        };
        let mut fence = VkFence::NULL;
        // SAFETY: `device` is live; `fence_info` is fully initialized; `&mut
        // fence` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_fence)(self.device(), &fence_info, ptr::null(), &mut fence)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateFence", result));
        }
        Ok(VulkanFence { fence })
    }

    unsafe fn destroy_fence(&self, fence: VulkanFence) {
        // SAFETY: `fence.fence` was created on this device, is not pending (caller
        // contract), and the by-value move destroys it exactly once.
        unsafe { (self.device_fns().destroy_fence)(self.device(), fence.fence, ptr::null()) };
    }

    fn wait_fence(&self, fence: &VulkanFence, timeout_ns: u64) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `&fence.fence` names one live fence;
        // `wait_all = VK_TRUE` blocks until it is signaled (or the timeout
        // elapses). After this returns `Ok` the submission that signals it has
        // completed — the fence-before-readback discipline.
        let raw = unsafe {
            (self.device_fns().wait_for_fences)(
                self.device(),
                1,
                &fence.fence,
                VK_TRUE,
                timeout_ns,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkWaitForFences", result));
        }
        Ok(())
    }

    fn reset_fence(&self, fence: &VulkanFence) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `&fence.fence` names one live fence to reset
        // to unsignaled (no submission referencing it is pending — caller resets
        // only after a `wait_fence`).
        let raw =
            unsafe { (self.device_fns().reset_fences)(self.device(), 1, &fence.fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkResetFences", result));
        }
        Ok(())
    }

    fn create_command_encoder(&self) -> Result<VulkanCommandEncoder, VulkanError> {
        let layouts = self.compute_layouts()?;
        // SAFETY: the device is live; `layouts` are this device's shared compute
        // layouts; the encoder takes a raw pointer to this context's `DeviceFns`
        // (which outlives any encoder built from `&self`).
        unsafe {
            VulkanCommandEncoder::new(
                self.device(),
                self.device_fns() as *const DeviceFns,
                self.queue_family_index(),
                layouts.set_layout,
                layouts.pipeline_layout,
            )
        }
    }

    unsafe fn destroy_command_encoder(&self, enc: VulkanCommandEncoder) {
        // SAFETY: `enc` was created on this device, its last submission has
        // completed (caller contract), and the by-value move destroys it exactly
        // once. `destroy` tears down the descriptor pool + command pool (which
        // frees the set + command buffer) in reverse order.
        unsafe { enc.destroy(self.device(), self.device_fns()) };
    }

    fn wait_idle(&self) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `vkDeviceWaitIdle` blocks until every queue is
        // idle — the belt-and-braces teardown sync (plan W4).
        let raw = unsafe { (self.device_fns().device_wait_idle)(self.device()) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkDeviceWaitIdle", result));
        }
        Ok(())
    }
}

impl RhiQueue<Vulkan> for VulkanQueue {
    type Error = VulkanError;

    fn submit(
        &self,
        encoder: &VulkanCommandEncoder,
        signal_fence: &VulkanFence,
    ) -> Result<(), VulkanError> {
        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: ptr::null(),
            p_wait_dst_stage_mask: ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &encoder.command_buffer,
            signal_semaphore_count: 0,
            p_signal_semaphores: ptr::null(),
        };
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns`
        // (stable heap address, context-outlives-queue — plan A1); the caller
        // upholds the type-level "context still alive" contract. `self.queue` is
        // its graphics+compute queue; one submit references the encoder's ended
        // `command_buffer` (the `&encoder.command_buffer` local outlives the call);
        // no semaphores (null arrays valid for count 0); `signal_fence.fence` is
        // the live fence signaled on completion. The headless path's only sync is
        // this fence.
        let fns = unsafe { &*self.fns };
        let raw = unsafe { (fns.queue_submit)(self.queue, 1, &submit, signal_fence.fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkQueueSubmit", result));
        }
        Ok(())
    }
}

impl VulkanCommandEncoder {
    /// Allocates the encoder's command pool + buffer + descriptor pool + the one
    /// fixed compute descriptor set (built ONCE here, plan Q1).
    ///
    /// On any partial failure every object created so far is torn down in reverse
    /// order before the error returns (the `ComputeHarness::new` rollback,
    /// narrowed to the per-encoder objects).
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the layouts belong to; `set_layout`
    /// / `pipeline_layout` must be that device's shared compute layouts; `fns`
    /// must outlive the returned encoder.
    unsafe fn new(
        device: VkDevice,
        fns: *const DeviceFns,
        queue_family_index: u32,
        set_layout: VkDescriptorSetLayout,
        pipeline_layout: VkPipelineLayout,
    ) -> Result<Self, VulkanError> {
        // SAFETY (whole fn): `fns` is a live `DeviceFns` borrowed from the owning
        // context (caller contract); dereferencing it here is sound on the owning
        // thread. Each create call below mirrors `ComputeHarness::new`'s sound
        // usage with the same `// SAFETY:` invariants.
        let fns_ref = unsafe { &*fns };

        // --- Descriptor pool + set (one STORAGE_BUFFER). ---
        let pool_size = VkDescriptorPoolSize {
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
        };
        let dp_info = VkDescriptorPoolCreateInfo {
            s_type: VkStructureType::DescriptorPoolCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            max_sets: 1,
            pool_size_count: 1,
            p_pool_sizes: &pool_size,
        };
        let mut descriptor_pool = VkDescriptorPool::NULL;
        // SAFETY: `device` is live; `dp_info` is fully initialized referencing the
        // `pool_size` local; `&mut descriptor_pool` is a valid out-pointer.
        let raw = unsafe {
            (fns_ref.create_descriptor_pool)(device, &dp_info, ptr::null(), &mut descriptor_pool)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateDescriptorPool", result));
        }

        let ds_alloc = VkDescriptorSetAllocateInfo {
            s_type: VkStructureType::DescriptorSetAllocateInfo,
            p_next: ptr::null(),
            descriptor_pool,
            descriptor_set_count: 1,
            p_set_layouts: &set_layout,
        };
        let mut descriptor_set = VkDescriptorSet::NULL;
        // SAFETY: `device` is live; `ds_alloc` names the live pool + references
        // the live `set_layout`; `&mut descriptor_set` is a valid out-pointer.
        let raw =
            unsafe { (fns_ref.allocate_descriptor_sets)(device, &ds_alloc, &mut descriptor_set) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `descriptor_pool` was just created; destroying it frees any
            // partially-allocated set and releases the pool exactly once.
            unsafe { (fns_ref.destroy_descriptor_pool)(device, descriptor_pool, ptr::null()) };
            return Err(VulkanError::Vk("vkAllocateDescriptorSets", result));
        }

        // --- Command pool + one primary command buffer. ---
        let cp_info = VkCommandPoolCreateInfo {
            s_type: VkStructureType::CommandPoolCreateInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index,
        };
        let mut command_pool = VkCommandPool::NULL;
        // SAFETY: `device` is live; `cp_info` is fully initialized for the
        // graphics+compute family; `&mut command_pool` is a valid out-pointer.
        let raw = unsafe {
            (fns_ref.create_command_pool)(device, &cp_info, ptr::null(), &mut command_pool)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: descriptor pool was created above; destroy it once on this
            // error path before returning.
            unsafe { (fns_ref.destroy_descriptor_pool)(device, descriptor_pool, ptr::null()) };
            return Err(VulkanError::Vk("vkCreateCommandPool", result));
        }

        let cb_alloc = VkCommandBufferAllocateInfo {
            s_type: VkStructureType::CommandBufferAllocateInfo,
            p_next: ptr::null(),
            command_pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut command_buffer = VkCommandBuffer::NULL;
        // SAFETY: `device` is live; `cb_alloc` names the live pool + requests one
        // primary buffer; `&mut command_buffer` is a valid out-pointer.
        let raw =
            unsafe { (fns_ref.allocate_command_buffers)(device, &cb_alloc, &mut command_buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: both pools were created above; destroy them once each in
            // reverse order on this error path.
            unsafe {
                (fns_ref.destroy_command_pool)(device, command_pool, ptr::null());
                (fns_ref.destroy_descriptor_pool)(device, descriptor_pool, ptr::null());
            }
            return Err(VulkanError::Vk("vkAllocateCommandBuffers", result));
        }

        Ok(Self {
            device,
            fns,
            command_pool,
            command_buffer,
            descriptor_pool,
            descriptor_set,
            pipeline_layout,
            bound_buffer: VkBuffer::NULL,
            bound_set_index: 0,
        })
    }

    /// Tears down the encoder's command pool + descriptor pool in reverse creation
    /// order, consuming `self`. The command buffer + descriptor set are freed
    /// implicitly by destroying their pools.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the encoder was created on; the
    /// encoder's last submission has completed (not pending); it is destroyed
    /// exactly once (the by-value `self` enforces the latter).
    unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
        // SAFETY: per the contract `device` is live and nothing is pending;
        // destroying the command pool frees its command buffer, and destroying the
        // descriptor pool frees its set — each pool destroyed exactly once in
        // reverse creation order.
        unsafe {
            (fns.destroy_command_pool)(device, self.command_pool, ptr::null());
            (fns.destroy_descriptor_pool)(device, self.descriptor_pool, ptr::null());
        }
    }
}

impl RhiCommandEncoder<Vulkan> for VulkanCommandEncoder {
    type Error = VulkanError;

    fn begin(&mut self) -> Result<(), VulkanError> {
        // Plan C1 (TD-1 ABA): reset the cached binding so every fresh recording
        // re-binds. `vkBeginCommandBuffer` resets the command buffer (so the prior
        // recording's `vkCmdBindDescriptorSets` is gone), and the descriptor set
        // itself may have been left pointing at a now-destroyed buffer whose
        // `VkBuffer` handle value a recreate could reuse — clearing the cache to
        // NULL forces a `vkUpdateDescriptorSets` on the next `bind_storage_buffer`,
        // closing the ABA while keeping the "at most one update per recording"
        // property (NULL never equals a real buffer handle).
        self.bound_buffer = VkBuffer::NULL;
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `self.fns` borrows the live device fn-table; the command buffer
        // is from a RESET_COMMAND_BUFFER pool, so `vkBeginCommandBuffer` implicitly
        // resets it (it is not pending — the caller fence-waits before reusing an
        // encoder); `begin` is a fully-initialized one-time-submit begin-info.
        let fns = unsafe { &*self.fns };
        let raw = unsafe { (fns.begin_command_buffer)(self.command_buffer, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkBeginCommandBuffer", result));
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), VulkanError> {
        // SAFETY: `self.fns` borrows the live device fn-table; recording was
        // opened by `begin`; `vkEndCommandBuffer` is its matching close.
        let fns = unsafe { &*self.fns };
        let raw = unsafe { (fns.end_command_buffer)(self.command_buffer) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkEndCommandBuffer", result));
        }
        Ok(())
    }

    fn bind_compute_pipeline(&mut self, pipeline: &ComputePipeline) {
        // SAFETY: recording is open; `pipeline.pipeline` is a live compute pipeline
        // built against this encoder's `pipeline_layout`; COMPUTE bind point
        // matches its creation. `self.fns` borrows the live device fn-table.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_pipeline)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                pipeline.pipeline,
            );
        }
    }

    fn bind_storage_buffer(&mut self, buffer: &BoundBuffer, set: u32, binding: u32) {
        // Plan B4 (ABI-4): the Slice-0 fixed layout is one STORAGE_BUFFER at
        // set0/binding0; any other `(set, binding)` is a caller error against the
        // fixed compute layout (Phase-6 bind groups supersede this).
        debug_assert!(
            set == 0 && binding == 0,
            "invariant: Slice-0 fixed set0/binding0"
        );
        self.bound_set_index = set;
        // Update the descriptor set ONLY when the bound buffer changes (plan Q1);
        // the foundation binds one buffer per recording, so the update fires at
        // most once. The actual `vkCmdBindDescriptorSets` is recorded at dispatch.
        if self.bound_buffer == buffer.buffer {
            return;
        }
        self.bound_buffer = buffer.buffer;

        let buffer_info = VkDescriptorBufferInfo {
            buffer: buffer.buffer,
            offset: 0,
            range: buffer.size,
        };
        let write = VkWriteDescriptorSet {
            s_type: VkStructureType::WriteDescriptorSet,
            p_next: ptr::null(),
            dst_set: self.descriptor_set,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            p_image_info: ptr::null(),
            p_buffer_info: &buffer_info,
            p_texel_buffer_view: ptr::null(),
        };
        // SAFETY: `self.fns` borrows the live device fn-table; one write references
        // the live `descriptor_set` + the `buffer_info` local (which names the
        // caller's live buffer); zero copies; the write is consumed entirely during
        // the call. The set is not bound to any pending command buffer (the caller
        // fence-waits before reusing the encoder), so updating it is sound.
        let fns = unsafe { &*self.fns };
        unsafe { (fns.update_descriptor_sets)(self.device, 1, &write, 0, ptr::null()) };
    }

    fn push_constants(&mut self, stage: ShaderStage, offset: u32, bytes: &[u8]) {
        // Plan B2 (ABI-1/TD-5): this encoder records exclusively against the shared
        // COMPUTE pipeline layout, whose push range is declared at offset 0 with
        // `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` bytes (P0a widened it from 4 to the
        // 80-byte `sdf_depth_composite` marcher block). `offset + len` outside
        // `[0, COMPUTE_PUSH_CONSTANT_RANGE_BYTES]`, or a non-COMPUTE stage, is a
        // caller error against that layout. Bound derived from the same constant the
        // layout uses, never a magic literal, so a future widening re-sizes both.
        debug_assert!(
            offset as u64 + bytes.len() as u64 <= COMPUTE_PUSH_CONSTANT_RANGE_BYTES as u64,
            "invariant: push range within COMPUTE_PUSH_CONSTANT_RANGE_BYTES"
        );
        debug_assert!(
            stage.bits() == crate::ffi::VK_SHADER_STAGE_COMPUTE_BIT,
            "invariant: compute push stage"
        );
        // The agnostic `ShaderStage` bits equal `VK_SHADER_STAGE_*` (plan D5).
        let stage_flags: VkFlags = stage.bits();
        // SAFETY: recording is open; `self.pipeline_layout` declares a
        // `COMPUTE_PUSH_CONSTANT_RANGE_BYTES`-byte COMPUTE push range at offset 0;
        // `bytes.as_ptr()` points to `bytes.len()` bytes alive for the call; the
        // caller passes offset/size within the declared range (asserted above).
        // `self.fns` borrows the live device fn-table.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_push_constants)(
                self.command_buffer,
                self.pipeline_layout,
                stage_flags,
                offset,
                bytes.len() as u32,
                bytes.as_ptr().cast::<c_void>(),
            );
        }
    }

    fn dispatch(&mut self, gx: u32, gy: u32, gz: u32) {
        debug_assert!(
            gx > 0 && gy > 0 && gz > 0,
            "invariant: dispatch group counts must be non-zero"
        );
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        // The packed-buffer path binds its STORAGE_BUFFER via `bind_storage_buffer`
        // (so `bound_buffer != NULL`) before every dispatch — for it the recorded
        // command stream is byte-identical to before: bind the fixed set against the
        // device-shared `pipeline_layout`, then dispatch. The Render P1a
        // vocabulary-compute path instead binds its set via
        // `bind_descriptor_set_compute` (against the pipeline's OWN layout) and never
        // calls `bind_storage_buffer` (`bound_buffer == NULL`), so the fixed-set rebind
        // is skipped — it would otherwise clobber the vocabulary set 0 and bind against
        // an incompatible layout. The two paths thus coexist without touching each
        // other's recorded commands.
        if self.bound_buffer != VkBuffer::NULL {
            // SAFETY: recording is open; the fixed STORAGE_BUFFER set was pointed at the
            // bound buffer by `bind_storage_buffer` and is bound at the cached set index
            // for the COMPUTE bind point against the live device-shared `pipeline_layout`;
            // zero dynamic offsets (null valid for count 0). `self.fns` borrows the live
            // device fn-table.
            unsafe {
                (fns.cmd_bind_descriptor_sets)(
                    self.command_buffer,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    self.pipeline_layout,
                    self.bound_set_index,
                    1,
                    &self.descriptor_set,
                    0,
                    ptr::null(),
                );
            }
        }
        // SAFETY: recording is open; the bound compute pipeline + its descriptor set
        // (the fixed set just bound above for the packed path, or the vocabulary set
        // bound earlier via `bind_descriptor_set_compute`) cover the dispatch.
        unsafe { (fns.cmd_dispatch)(self.command_buffer, gx, gy, gz) };
    }

    fn pipeline_barrier(&mut self, barrier: &BarrierDesc<Vulkan>) {
        // Map the agnostic stage/access masks via identity casts — the
        // `BarrierStage`/`BarrierAccess` bit values equal the `VK_PIPELINE_STAGE_*`
        // / `VK_ACCESS_*` constants (plan D3/D5).
        let src_stage: VkFlags = barrier.src_stage.bits();
        let dst_stage: VkFlags = barrier.dst_stage.bits();
        debug_assert!(
            barrier.buffers.is_empty() || (src_stage != 0 && dst_stage != 0),
            "invariant: a buffer barrier needs non-empty src+dst stages"
        );

        // The foundation supplies 0 or 1 buffer barriers — the common, hot path.
        // Plan D1 (TD-3/UB-4): the multi-barrier heap fallback (never hit on the
        // headless compute path) is factored into a `#[cold]` helper so this path
        // never even names a `Vec`.
        let count = barrier.buffers.len();
        if count <= 1 {
            let mut inline_buf = VkBufferMemoryBarrier {
                s_type: VkStructureType::BufferMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: 0,
                dst_access_mask: 0,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                buffer: VkBuffer::NULL,
                offset: 0,
                size: VK_WHOLE_SIZE,
            };
            let vk_barriers: *const VkBufferMemoryBarrier = if count == 0 {
                ptr::null()
            } else {
                let b = &barrier.buffers[0];
                inline_buf.src_access_mask = b.src_access.bits();
                inline_buf.dst_access_mask = b.dst_access.bits();
                inline_buf.buffer = b.buffer.buffer;
                &inline_buf
            };
            // SAFETY: recording is open; `src_stage`/`dst_stage` are the mapped
            // stage masks; `vk_barriers` points to `count` (0 or 1) fully-
            // initialized `VkBufferMemoryBarrier`s in the live `inline_buf` local,
            // naming a live buffer with WRITE→READ|WRITE-style scopes; zero
            // global/image barriers (null arrays valid for count 0). `self.fns`
            // points into the context's boxed fn-table (alive per the type
            // contract).
            let fns = unsafe { &*self.fns };
            unsafe {
                (fns.cmd_pipeline_barrier)(
                    self.command_buffer,
                    src_stage,
                    dst_stage,
                    0,
                    0,
                    ptr::null(),
                    count as u32,
                    vk_barriers,
                    0,
                    ptr::null(),
                );
            }
            return;
        }

        self.pipeline_barrier_many(src_stage, dst_stage, barrier.buffers);
    }

    fn copy_buffer(&mut self, src: &BoundBuffer, dst: &BoundBuffer, regions: &[BufferCopy]) {
        debug_assert!(!regions.is_empty(), "invariant: copy_buffer needs >= 1 region");
        // The agnostic `BufferCopy` is `#[repr(C)] { src_offset, dst_offset, size:
        // u64 }` — byte-identical to the Vulkan `VkBufferCopy` (same field order +
        // `u64` types), so a `&[BufferCopy]` reinterprets directly as a
        // `&[VkBufferCopy]` without a per-region copy. The size + alignment match
        // is enforced at build time here.
        const _: () = assert!(
            core::mem::size_of::<BufferCopy>() == core::mem::size_of::<VkBufferCopy>(),
            "BufferCopy and VkBufferCopy must share size for the slice reinterpret"
        );
        const _: () = assert!(
            core::mem::align_of::<BufferCopy>() == core::mem::align_of::<VkBufferCopy>(),
            "BufferCopy and VkBufferCopy must share alignment for the slice reinterpret"
        );
        // SAFETY: `BufferCopy` and `VkBufferCopy` are both `#[repr(C)]` with the
        // identical `(u64, u64, u64)` layout (size + align asserted above), so
        // casting the `*const BufferCopy` to `*const VkBufferCopy` and reading
        // `regions.len()` elements is in-bounds and ABI-valid — every field maps
        // 1:1. The slice is alive for the call.
        let vk_regions = regions.as_ptr().cast::<VkBufferCopy>();
        // SAFETY: recording is open; `src.buffer`/`dst.buffer` are live buffers
        // (created on this device, carrying the `TRANSFER_SRC`/`TRANSFER_DST` usage
        // the device-local path always adds); `vk_regions` points to `regions.len()`
        // fully-initialized `VkBufferCopy`s alive for the call. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_copy_buffer)(
                self.command_buffer,
                src.buffer,
                dst.buffer,
                regions.len() as u32,
                vk_regions,
            );
        }
    }

    fn image_barrier(&mut self, barrier: &ImageBarrierDesc<Vulkan>) {
        // Map the agnostic stage/access masks via identity casts (the
        // `BarrierStage`/`BarrierAccess` bit values equal the `VK_PIPELINE_STAGE_*`
        // / `VK_ACCESS_*` constants — asserted in `abi_guard.rs`); `ImageLayout` /
        // `ImageAspect` are the `i32`/`u32` FFI families mapped by `as_i32()`/
        // `bits()`. This abstracts the concrete `swapchain.rs::record_clear`
        // `VkImageMemoryBarrier`.
        let src_stage: VkFlags = barrier.src_stage.bits();
        let dst_stage: VkFlags = barrier.dst_stage.bits();
        debug_assert!(
            src_stage != 0 && dst_stage != 0,
            "invariant: an image barrier needs non-empty src+dst stages"
        );
        let image_barrier = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: barrier.src_access.bits(),
            dst_access_mask: barrier.dst_access.bits(),
            old_layout: barrier.old_layout.as_i32(),
            new_layout: barrier.new_layout.as_i32(),
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: barrier.texture.image,
            subresource_range: VkImageSubresourceRange {
                aspect_mask: barrier.range.aspect.bits(),
                base_mip_level: barrier.range.base_mip_level,
                level_count: barrier.range.level_count,
                base_array_layer: barrier.range.base_array_layer,
                layer_count: barrier.range.layer_count,
            },
        };
        // SAFETY: recording is open; `src_stage`/`dst_stage` are the mapped stage
        // masks; one fully-initialized `VkImageMemoryBarrier` (the `image_barrier`
        // local, alive for the call) names the live `barrier.texture.image` with the
        // requested old→new layout + access scopes; zero global/buffer barriers
        // (null arrays valid for count 0). `self.fns` points into the context's
        // boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_pipeline_barrier)(
                self.command_buffer,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&image_barrier as *const VkImageMemoryBarrier).cast(),
            );
        }
    }

    fn begin_rendering(&mut self, desc: &RenderingDesc<Vulkan>) {
        // Abstracts the concrete `swapchain.rs::record_clear` `VkRenderingInfo`
        // begin (one color attachment, loadOp/storeOp/clear from the desc). The
        // basic slice's G-buffer has a small, fixed attachment count, so the
        // attachment array is a stack local sized for `MAX_RENDERING_COLOR_ATTACHMENTS`
        // — zero heap allocation on the record path.
        let count = desc.colors.len();
        debug_assert!(
            count <= MAX_RENDERING_COLOR_ATTACHMENTS,
            "invariant: begin_rendering color-attachment count exceeds the fixed cap"
        );
        let count = count.min(MAX_RENDERING_COLOR_ATTACHMENTS);

        // Build the fixed-capacity attachment array: the first `count` slots map the
        // caller's color attachments; the tail slots hold a neutral default that is
        // never read (only `count` entries are passed to the driver). `from_fn`
        // avoids requiring `Copy` on the raw-pointer-bearing `VkRenderingAttachmentInfo`.
        let attachments: [VkRenderingAttachmentInfo; MAX_RENDERING_COLOR_ATTACHMENTS] =
            core::array::from_fn(|i| {
                if i < count {
                    let color = &desc.colors[i];
                    VkRenderingAttachmentInfo {
                        s_type: VkStructureType::RenderingAttachmentInfo,
                        p_next: ptr::null(),
                        image_view: color.texture.view,
                        image_layout: color.layout.as_i32(),
                        resolve_mode: 0,
                        resolve_image_view: VkImageView::NULL,
                        resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        load_op: color.load_op.as_i32(),
                        store_op: color.store_op.as_i32(),
                        clear_value: VkClearValue {
                            color: VkClearColorValue {
                                float32: color.clear_color,
                            },
                        },
                    }
                } else {
                    VkRenderingAttachmentInfo {
                        s_type: VkStructureType::RenderingAttachmentInfo,
                        p_next: ptr::null(),
                        image_view: VkImageView::NULL,
                        image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        resolve_mode: 0,
                        resolve_image_view: VkImageView::NULL,
                        resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                        load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                        store_op: VK_ATTACHMENT_STORE_OP_STORE,
                        clear_value: VkClearValue {
                            color: VkClearColorValue { float32: [0.0; 4] },
                        },
                    }
                }
            });

        // The optional depth attachment (Phase-6 S0 rung 4). When present, build one
        // `VkRenderingAttachmentInfo` whose clear value uses the depth-stencil variant
        // of the `VkClearValue` union (depth = `clear_depth`, e.g. 1.0; stencil unused).
        // The `depth_attachment` local must outlive the `cmd_begin_rendering` call, so
        // it is bound here and `p_depth_attachment` points at it; `None` leaves the
        // pointer null (the rungs-1..3 no-depth path). `as_i32()` lowerings equal the
        // `VkImageLayout`/`VkAttachmentLoadOp`/`VkAttachmentStoreOp` constants (asserted
        // in `abi_guard.rs`).
        let depth_attachment = desc.depth.as_ref().map(|d| VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: d.texture.view,
            image_layout: d.layout.as_i32(),
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: d.load_op.as_i32(),
            store_op: d.store_op.as_i32(),
            clear_value: VkClearValue {
                depth_stencil: VkClearDepthStencilValue {
                    depth: d.clear_depth,
                    stencil: 0,
                },
            },
        });
        let p_depth_attachment: *const c_void = match &depth_attachment {
            Some(att) => (att as *const VkRenderingAttachmentInfo).cast(),
            None => ptr::null(),
        };

        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D {
                offset: VkOffset2D {
                    x: desc.render_area.x,
                    y: desc.render_area.y,
                },
                extent: VkExtent2D {
                    width: desc.render_area.width,
                    height: desc.render_area.height,
                },
            },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: count as u32,
            p_color_attachments: if count == 0 {
                ptr::null()
            } else {
                attachments.as_ptr()
            },
            p_depth_attachment,
            p_stencil_attachment: ptr::null(),
        };
        // SAFETY: recording is open; `rendering` is fully initialized and its
        // `p_color_attachments` points to the first `count` entries of the live
        // `attachments` stack array (each naming the caller's live image view, now
        // in the declared layout per a prior `image_barrier`). `p_depth_attachment`
        // points at the live `depth_attachment` local (alive for this call) naming the
        // caller's live DEPTH-aspect view in DEPTH_ATTACHMENT_OPTIMAL (per a prior
        // depth `image_barrier`) when a depth attachment is requested, else null. No
        // stencil (null). Dynamic rendering is enabled on the device (`dynamicRendering`
        // feature, Correction #1). All locals outlive the call. `self.fns` points into
        // the context's boxed fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_begin_rendering)(self.command_buffer, &rendering) };
    }

    fn end_rendering(&mut self) {
        // SAFETY: recording is open and a `begin_rendering` opened the scope (caller
        // contract); `vkCmdEndRendering` is its matching close. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_end_rendering)(self.command_buffer) };
    }

    fn bind_graphics_pipeline(&mut self, pipeline: &VulkanGraphicsPipeline) {
        // SAFETY: recording is open; `pipeline.pipeline` is a live graphics pipeline
        // (its declared color format must match the enclosing `begin_rendering`
        // scope — the W2-b draw-time contract); the GRAPHICS bind point matches its
        // creation. `self.fns` points into the context's boxed fn-table (alive per
        // the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_pipeline)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                pipeline.pipeline,
            );
        }
    }

    fn bind_descriptor_set(
        &mut self,
        group: &VulkanBindGroup,
        pipeline: &VulkanGraphicsPipeline,
    ) {
        // SAFETY: recording is open and inside a `begin_rendering` scope with the
        // matching graphics pipeline bound (caller contract); `pipeline.layout` is
        // that pipeline's own layout, built with the same bind-group set-layout at
        // `set 0` (`GraphicsPipelineDesc::bind_group_layout`), so binding
        // `group.descriptor_set` there for the GRAPHICS bind point is type-compatible.
        // `&group.descriptor_set` is a single-element local (alive for the call), so
        // `first_set = 0`, `descriptor_set_count = 1` matches it; zero dynamic offsets
        // (null valid for count 0). `self.fns` points into the context's boxed
        // fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_descriptor_sets)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                pipeline.layout,
                0,
                1,
                &group.descriptor_set,
                0,
                ptr::null(),
            );
        }
    }

    fn bind_descriptor_set_compute(
        &mut self,
        group: &VulkanBindGroup,
        compute_pipeline: &ComputePipeline,
    ) {
        // SAFETY: recording is open with `compute_pipeline` bound (caller contract);
        // `compute_pipeline.layout` is that pipeline's own layout, built with the same
        // vocabulary bind-group set-layout at `set 0`
        // (`ComputePipelineDesc::bind_group_layout`), so binding `group.descriptor_set`
        // there for the COMPUTE bind point is type-compatible. `&group.descriptor_set`
        // is a single-element local (alive for the call), so `first_set = 0`,
        // `descriptor_set_count = 1` matches it; zero dynamic offsets (null valid for
        // count 0). This binds the vocabulary set ONLY — it does not touch the
        // encoder's fixed STORAGE_BUFFER set (`bind_storage_buffer`/`dispatch`), so the
        // packed-buffer offscreen path is unaffected. `self.fns` points into the
        // context's boxed fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_descriptor_sets)(
                self.command_buffer,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                compute_pipeline.layout,
                0,
                1,
                &group.descriptor_set,
                0,
                ptr::null(),
            );
        }
    }

    fn set_viewport(&mut self, viewport: &Viewport) {
        // The agnostic `Viewport` is `#[repr(C)] { x, y, width, height, min_depth,
        // max_depth: f32 }` — byte-identical to `VkViewport` (same field order +
        // `f32` types), so the `*const Viewport` casts directly to `*const VkViewport`
        // without a per-call copy. The size + align match is enforced at build time.
        const _: () = assert!(
            core::mem::size_of::<Viewport>() == core::mem::size_of::<VkViewport>(),
            "Viewport and VkViewport must share size for the pointer reinterpret"
        );
        const _: () = assert!(
            core::mem::align_of::<Viewport>() == core::mem::align_of::<VkViewport>(),
            "Viewport and VkViewport must share alignment for the pointer reinterpret"
        );
        let vk_viewport = (viewport as *const Viewport).cast::<VkViewport>();
        // SAFETY: recording is open; `Viewport`/`VkViewport` share layout (asserted),
        // so reading one `VkViewport` from `vk_viewport` (the live `viewport` borrow,
        // alive for the call) is ABI-valid; `first_viewport = 0`, `count = 1` matches
        // the pipeline's single dynamic viewport. `self.fns` points into the context's
        // boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_set_viewport)(self.command_buffer, 0, 1, vk_viewport) };
    }

    fn set_scissor(&mut self, scissor: &RenderArea) {
        let rect = VkRect2D {
            offset: VkOffset2D {
                x: scissor.x,
                y: scissor.y,
            },
            extent: VkExtent2D {
                width: scissor.width,
                height: scissor.height,
            },
        };
        // SAFETY: recording is open; one fully-initialized `VkRect2D` (the `rect`
        // local, alive for the call) describes the scissor; `first_scissor = 0`,
        // `count = 1` matches the pipeline's single dynamic scissor. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe { (fns.cmd_set_scissor)(self.command_buffer, 0, 1, &rect) };
    }

    fn bind_vertex_buffer(&mut self, buffer: &BoundBuffer, binding: u32, offset: u64) {
        let buffers = [buffer.buffer];
        let offsets = [offset as VkDeviceSize];
        // SAFETY: recording is open; `buffer.buffer` is a live buffer (created on this
        // device, carrying VERTEX usage); `buffers`/`offsets` are single-element stack
        // locals alive for the call, so `binding_count = 1` matches both array
        // pointers; `offset` is a byte offset within the bound buffer (the caller's
        // contract). `self.fns` points into the context's boxed fn-table (alive per
        // the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_vertex_buffers)(
                self.command_buffer,
                binding,
                1,
                buffers.as_ptr(),
                offsets.as_ptr(),
            );
        }
    }

    fn bind_index_buffer(&mut self, buffer: &BoundBuffer, offset: u64, index_type: IndexType) {
        // The agnostic `IndexType` discriminant equals the `VkIndexType` constant
        // (asserted in `abi_guard.rs`).
        // SAFETY: recording is open; `buffer.buffer` is a live buffer (created on this
        // device, carrying INDEX usage); `offset` is a byte offset within it (the
        // caller's contract); `index_type` is a valid `VkIndexType`. `self.fns` points
        // into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_bind_index_buffer)(
                self.command_buffer,
                buffer.buffer,
                offset as VkDeviceSize,
                index_type.as_i32(),
            );
        }
    }

    fn push_graphics_constants(
        &mut self,
        pipeline: &VulkanGraphicsPipeline,
        stage: ShaderStage,
        offset: u32,
        bytes: &[u8],
    ) {
        // The agnostic `ShaderStage` bits equal `VK_SHADER_STAGE_*` (plan D5,
        // asserted in `abi_guard.rs`).
        let stage_flags: VkFlags = stage.bits();
        // SAFETY: recording is open; `pipeline.layout` is the graphics pipeline's own
        // layout (created in `create_graphics_pipeline` with a VERTEX-stage push range
        // at offset 0). The encoder does NOT carry the layout's declared push size, so
        // it cannot statically bound `stage`/`offset`/`bytes` against it — an over-range
        // or wrong-stage push is caught at runtime by the Vulkan validation layer (the
        // GPU-half soundness oracle), not by a debug_assert here (contrast the compute
        // sibling, whose FIXED 4-byte/COMPUTE layout makes a static assert trivial).
        // `bytes.as_ptr()` points to `bytes.len()` bytes alive for the call; `self.fns`
        // points into the context's boxed fn-table (alive per the type contract).
        // SAFETY: `self.fns` points into the owning context's boxed `DeviceFns` — a stable
        // heap address that outlives this encoder (context teardown order); deref is valid.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_push_constants)(
                self.command_buffer,
                pipeline.layout,
                stage_flags,
                offset,
                bytes.len() as u32,
                bytes.as_ptr().cast::<c_void>(),
            );
        }
    }

    fn push_compute_constants(
        &mut self,
        pipeline: &ComputePipeline,
        stage: ShaderStage,
        offset: u32,
        bytes: &[u8],
    ) {
        // Render P4b: the COMPUTE counterpart of `push_graphics_constants`. Pushes
        // against the passed pipeline's OWN layout — for a vocabulary pipeline
        // (`ComputePipelineDesc::bind_group_layout == Some`) that is the DEDICATED
        // layout its bind group was bound against (`bind_descriptor_set_compute`),
        // which declares a `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` COMPUTE range at offset 0
        // (see `create_compute_pipeline`). The fine marcher pushes a 4-byte
        // `coarse_enabled` gate here.
        let stage_flags: VkFlags = stage.bits();
        debug_assert!(
            stage_flags == crate::ffi::VK_SHADER_STAGE_COMPUTE_BIT,
            "invariant: compute push stage"
        );
        debug_assert!(
            offset as u64 + bytes.len() as u64 <= COMPUTE_PUSH_CONSTANT_RANGE_BYTES as u64,
            "invariant: push range within the pipeline's COMPUTE push range"
        );
        // SAFETY: recording is open with `pipeline` bound (caller contract);
        // `pipeline.layout` is that compute pipeline's own layout, which declares a
        // `COMPUTE_PUSH_CONSTANT_RANGE_BYTES`-byte COMPUTE push range at offset 0
        // (created in `create_compute_pipeline`); `offset`/`bytes.len()` are within that
        // range (asserted above) at the COMPUTE stage; `bytes.as_ptr()` points to
        // `bytes.len()` bytes alive for the call. `self.fns` borrows the live device
        // fn-table.
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_push_constants)(
                self.command_buffer,
                pipeline.layout,
                stage_flags,
                offset,
                bytes.len() as u32,
                bytes.as_ptr().cast::<c_void>(),
            );
        }
    }

    fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        // A zero `vertex_count`/`instance_count` is a legal Vulkan no-op — a culled or
        // GPU-driven-indirect path may legitimately issue one — so the RHI deliberately
        // permits it rather than asserting non-zero (API-faithful for the future
        // indirect/culled draw rungs).
        // SAFETY: recording is open and inside a `begin_rendering` scope with a bound
        // graphics pipeline + a set dynamic viewport/scissor (caller contract);
        // `vkCmdDraw` issues the non-indexed draw. `self.fns` points into the
        // context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_draw)(
                self.command_buffer,
                vertex_count,
                instance_count,
                first_vertex,
                first_instance,
            );
        }
    }

    fn draw_indexed(
        &mut self,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        // A zero `index_count`/`instance_count` is a legal Vulkan no-op (as in `draw`)
        // — a culled or GPU-driven-indirect path may legitimately issue one — so the
        // RHI deliberately permits it rather than asserting non-zero.
        // SAFETY: recording is open and inside a `begin_rendering` scope with a bound
        // graphics pipeline + a set dynamic viewport/scissor, a bound index buffer
        // (`bind_index_buffer`) and the vertex buffer(s) the indices reference (caller
        // contract); `vkCmdDrawIndexed` reads `index_count` indices from `first_index`
        // in the bound index buffer, adds `vertex_offset` per index, and issues the
        // indexed draw. `self.fns` points into the context's boxed fn-table (alive per
        // the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_draw_indexed)(
                self.command_buffer,
                index_count,
                instance_count,
                first_index,
                vertex_offset,
                first_instance,
            );
        }
    }

    fn copy_image_to_buffer(
        &mut self,
        src: &VulkanTexture,
        src_layout: ImageLayout,
        dst: &BoundBuffer,
        regions: &[BufferImageCopy],
    ) {
        debug_assert!(
            !regions.is_empty(),
            "invariant: copy_image_to_buffer needs >= 1 region"
        );
        // The basic-slice readback uses a single full-image region; the inline cap
        // avoids any heap allocation on that path. A larger batch (never hit by S0)
        // falls into the cold heap helper, mirroring `pipeline_barrier_many`.
        if regions.len() <= MAX_IMAGE_COPY_REGIONS {
            // Invariant (mirrors `begin_rendering`'s belt-and-suspenders): inside this
            // branch the count is provably `<= MAX_IMAGE_COPY_REGIONS`, so the
            // `inline_regions[..regions.len()]` slice fill is in-bounds and the
            // `regions.len() as u32` region count handed to Vulkan is `<= CAP`. The
            // `> CAP` case is routed to the cold heap helper below, never here — this
            // assert traps any future refactor that loosens the branch condition.
            debug_assert!(regions.len() <= MAX_IMAGE_COPY_REGIONS);
            let mut inline_regions = [DEFAULT_BUFFER_IMAGE_COPY; MAX_IMAGE_COPY_REGIONS];
            for (slot, region) in inline_regions.iter_mut().zip(regions.iter()) {
                *slot = vk_buffer_image_copy(region);
            }
            // SAFETY: recording is open; `src.image` is a live image currently in
            // `src_layout` (the caller transitioned it via `image_barrier`);
            // `dst.buffer` is a live host-visible buffer carrying TRANSFER_DST usage;
            // `inline_regions[..regions.len()]` are fully-initialized `VkBufferImageCopy`s
            // (alive for the call) describing in-bounds sub-rects. `self.fns` points
            // into the context's boxed fn-table (alive per the type contract).
            let fns = unsafe { &*self.fns };
            unsafe {
                (fns.cmd_copy_image_to_buffer)(
                    self.command_buffer,
                    src.image,
                    src_layout.as_i32(),
                    dst.buffer,
                    regions.len() as u32,
                    inline_regions.as_ptr(),
                );
            }
            return;
        }
        self.copy_image_to_buffer_many(src.image, src_layout.as_i32(), dst.buffer, regions);
    }

    fn copy_buffer_to_image(
        &mut self,
        src: &BoundBuffer,
        dst: &VulkanTexture,
        dst_layout: ImageLayout,
        regions: &[BufferImageCopy],
    ) {
        debug_assert!(
            !regions.is_empty(),
            "invariant: copy_buffer_to_image needs >= 1 region"
        );
        // The rung-11 composite upload uses a single full-image region; the inline
        // cap avoids any heap allocation on that path. A larger batch (never hit by
        // S1) falls into the cold heap helper, mirroring `copy_image_to_buffer`.
        if regions.len() <= MAX_IMAGE_COPY_REGIONS {
            // Invariant (mirrors `copy_image_to_buffer`): inside this branch the count
            // is provably `<= MAX_IMAGE_COPY_REGIONS`, so the `inline_regions[..len]`
            // fill is in-bounds and the `len as u32` count handed to Vulkan is `<=
            // CAP`. The `> CAP` case is routed to the cold heap helper below — this
            // assert traps any future refactor that loosens the branch condition.
            debug_assert!(regions.len() <= MAX_IMAGE_COPY_REGIONS);
            let mut inline_regions = [DEFAULT_BUFFER_IMAGE_COPY; MAX_IMAGE_COPY_REGIONS];
            for (slot, region) in inline_regions.iter_mut().zip(regions.iter()) {
                *slot = vk_buffer_image_copy(region);
            }
            // SAFETY: recording is open; `src.buffer` is a live buffer carrying
            // TRANSFER_SRC usage (the host-coherent composite buffer); `dst.image` is
            // a live image currently in `dst_layout` (the caller transitioned it to
            // TRANSFER_DST_OPTIMAL via `image_barrier`);
            // `inline_regions[..regions.len()]` are fully-initialized
            // `VkBufferImageCopy`s (alive for the call) describing in-bounds sub-rects.
            // `self.fns` points into the context's boxed fn-table (alive per the type
            // contract).
            let fns = unsafe { &*self.fns };
            unsafe {
                (fns.cmd_copy_buffer_to_image)(
                    self.command_buffer,
                    src.buffer,
                    dst.image,
                    dst_layout.as_i32(),
                    regions.len() as u32,
                    inline_regions.as_ptr(),
                );
            }
            return;
        }
        self.copy_buffer_to_image_many(src.buffer, dst.image, dst_layout.as_i32(), regions);
    }
}

impl VulkanCommandEncoder {
    /// Records a `vkCmdClearColorImage` over `range` of `texture` (which the caller MUST
    /// have transitioned to `layout`, one of `GENERAL`/`TRANSFER_DST_OPTIMAL`), clearing
    /// every covered texel to `color` (SDFDDGI I1 boot-clear of the probe atlases). A
    /// crate-internal helper (not on the public `RhiCommandEncoder` trait) reaching the
    /// encoder's private `command_buffer`/`fns` the same way [`Self::image_barrier`] does.
    pub(crate) fn clear_color_image(
        &mut self,
        texture: &VulkanTexture,
        layout: ImageLayout,
        color: [f32; 4],
        range: ImageSubresourceRange,
    ) {
        let clear = VkClearColorValue { float32: color };
        let vk_range = VkImageSubresourceRange {
            aspect_mask: range.aspect.bits(),
            base_mip_level: range.base_mip_level,
            level_count: range.level_count,
            base_array_layer: range.base_array_layer,
            layer_count: range.layer_count,
        };
        // SAFETY: recording is open; `texture.image` is a live COLOR image the caller has
        // transitioned to `layout` (TRANSFER_DST_OPTIMAL per its clear boot path);
        // `&clear` + `&vk_range` are fully-initialized locals alive for the call, and
        // `vk_range` names an in-bounds subresource (the caller passes the image's own
        // `0..layer_count`). `self.fns` points into the context's boxed fn-table (alive
        // per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_clear_color_image)(
                self.command_buffer,
                texture.image,
                layout.as_i32(),
                &clear,
                1,
                &vk_range,
            );
        }
    }

    /// Records a `vkCmdFillBuffer` filling all `size` bytes of `buffer` from offset 0 with
    /// the 4-byte `pattern` (SDFDDGI I1 boot-clear of the per-probe classification buffer
    /// to 0 = unconverged). A crate-internal helper reaching the encoder's private
    /// `command_buffer`/`fns` directly, mirroring the gbuffer cull's `cmd_fill_buffer` reset.
    pub(crate) fn fill_buffer(&mut self, buffer: &BoundBuffer, pattern: u32) {
        // SAFETY: recording is open; `buffer.buffer` is a live buffer carrying TRANSFER_DST
        // usage (the classification buffer is created with it); `buffer.size` is its exact
        // byte size (a multiple of 4 — a `u8`-per-probe count rounded to a `u32` word).
        // `self.fns` points into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_fill_buffer)(self.command_buffer, buffer.buffer, 0, buffer.size, pattern);
        }
    }

    /// The cold multi-buffer-barrier fallback for [`RhiCommandEncoder::pipeline_barrier`]
    /// (plan D1): builds a heap `Vec<VkBufferMemoryBarrier>` and records the
    /// barrier. The headless compute path never reaches this (it supplies 0 or 1
    /// buffer barriers), so the only allocation is kept off the common path's
    /// I-cache via `#[cold] #[inline(never)]`.
    #[cold]
    #[inline(never)]
    fn pipeline_barrier_many(
        &mut self,
        src_stage: VkFlags,
        dst_stage: VkFlags,
        buffers: &[BufferBarrier<Vulkan>],
    ) {
        let mut heap_buf: Vec<VkBufferMemoryBarrier> = Vec::with_capacity(buffers.len());
        for b in buffers {
            heap_buf.push(VkBufferMemoryBarrier {
                s_type: VkStructureType::BufferMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: b.src_access.bits(),
                dst_access_mask: b.dst_access.bits(),
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                buffer: b.buffer.buffer,
                offset: 0,
                size: VK_WHOLE_SIZE,
            });
        }
        // SAFETY: recording is open; `src_stage`/`dst_stage` are the mapped stage
        // masks; `heap_buf` holds `buffers.len()` fully-initialized
        // `VkBufferMemoryBarrier`s (alive for the call), each naming a live buffer;
        // zero global/image barriers (null arrays valid for count 0). `self.fns`
        // points into the context's boxed fn-table (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_pipeline_barrier)(
                self.command_buffer,
                src_stage,
                dst_stage,
                0,
                0,
                ptr::null(),
                heap_buf.len() as u32,
                heap_buf.as_ptr(),
                0,
                ptr::null(),
            );
        }
    }

    /// The cold multi-region fallback for [`RhiCommandEncoder::copy_image_to_buffer`]:
    /// builds a heap `Vec<VkBufferImageCopy>` and records the copy. The basic-slice
    /// readback uses a single region, so this path (and its only allocation) is kept
    /// off the common path's I-cache via `#[cold] #[inline(never)]`.
    #[cold]
    #[inline(never)]
    fn copy_image_to_buffer_many(
        &mut self,
        src_image: VkImage,
        src_layout: i32,
        dst_buffer: VkBuffer,
        regions: &[BufferImageCopy],
    ) {
        let mut heap_regions: Vec<VkBufferImageCopy> = Vec::with_capacity(regions.len());
        for r in regions {
            heap_regions.push(vk_buffer_image_copy(r));
        }
        // SAFETY: recording is open; `src_image` is a live image in `src_layout`;
        // `dst_buffer` is a live TRANSFER_DST buffer; `heap_regions` holds
        // `regions.len()` fully-initialized `VkBufferImageCopy`s alive for the call.
        // `self.fns` points into the context's boxed fn-table (alive per the type
        // contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_copy_image_to_buffer)(
                self.command_buffer,
                src_image,
                src_layout,
                dst_buffer,
                heap_regions.len() as u32,
                heap_regions.as_ptr(),
            );
        }
    }

    /// The cold multi-region fallback for [`RhiCommandEncoder::copy_buffer_to_image`]:
    /// builds a heap `Vec<VkBufferImageCopy>` and records the copy. The rung-11
    /// composite upload uses a single region, so this path (and its only allocation)
    /// is kept off the common path's I-cache via `#[cold] #[inline(never)]`
    /// (mirrors [`Self::copy_image_to_buffer_many`]).
    #[cold]
    #[inline(never)]
    fn copy_buffer_to_image_many(
        &mut self,
        src_buffer: VkBuffer,
        dst_image: VkImage,
        dst_layout: i32,
        regions: &[BufferImageCopy],
    ) {
        let mut heap_regions: Vec<VkBufferImageCopy> = Vec::with_capacity(regions.len());
        for r in regions {
            heap_regions.push(vk_buffer_image_copy(r));
        }
        // SAFETY: recording is open; `src_buffer` is a live TRANSFER_SRC buffer;
        // `dst_image` is a live image in `dst_layout` (TRANSFER_DST_OPTIMAL);
        // `heap_regions` holds `regions.len()` fully-initialized `VkBufferImageCopy`s
        // alive for the call. `self.fns` points into the context's boxed fn-table
        // (alive per the type contract).
        let fns = unsafe { &*self.fns };
        unsafe {
            (fns.cmd_copy_buffer_to_image)(
                self.command_buffer,
                src_buffer,
                dst_image,
                dst_layout,
                heap_regions.len() as u32,
                heap_regions.as_ptr(),
            );
        }
    }
}
