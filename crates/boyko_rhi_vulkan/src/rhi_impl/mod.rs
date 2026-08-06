//! The [`boyko_rhi`] trait implementation for the Vulkan backend — compute path
//! only (Phase 1, plan Waves C+D).
//!
//! [`Vulkan`] is the zero-sized [`RhiApi`] marker. [`VulkanContext`] implements
//! [`RhiDevice`](boyko_rhi::RhiDevice); a thin [`VulkanQueue`] implements [`RhiQueue`] (plan O1/Q2);
//! [`VulkanCommandEncoder`] implements [`RhiCommandEncoder`](boyko_rhi::RhiCommandEncoder) (the hot recording
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

use core::ptr;
#[cfg(feature = "hwrt")]
use core::ffi::c_void;

use boyko_rhi::{BindGroupEntry, BufferImageCopy, DescriptorKind, RhiApi, RhiQueue};

#[cfg(feature = "hwrt")]
use crate::accel::BoundAccelStruct;
#[cfg(feature = "hwrt")]
use crate::accel_ffi::{
    ST_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR, VkWriteDescriptorSetAccelerationStructureKHR,
};
use crate::device::{DeviceFns, VulkanContext};
use crate::error::VulkanError;
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::texture::{VulkanTexture, VulkanTextureView};

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
/// reserve room for the 3 DDGI resolve bindings landed in rung I0. HW-RT rung R2a-4a
/// raised it 19 → 20 to reserve binding 19 for the resolve's
/// `RaytracingAccelerationStructure` (the TLAS the rayQuery mesh-shadow trace reads) —
/// BYTE-NEUTRAL: the software resolve still fills 19, only the inline-array capacity
/// grows. HW-RT rung 1b raised it 20 → 21 to reserve binding 20 for the HWRT resolve's
/// tunable soft-shadow params UBO (`boyko_render::ResolvedRayShadow`) — BYTE-NEUTRAL by
/// the same argument: only the HWRT set fills the new tail slot, the software resolve
/// still fills 19. HW-RT rung 3a raised it 21 → 22 to reserve binding 21 for the VIS/DENOISED
/// resolve variants' `gShadowVis` UAV (`RWTexture2D<float2>`) — BYTE-NEUTRAL by the same
/// argument: only the 22-binding VIS/DENOISED layout fills the new tail slot, the software
/// resolve still fills 19 and the RESOLVE_INLINE-hwrt resolve still fills 21. HW-RT rung 3b step 5b
/// raised it 22 → 24 to reserve bindings 22/23 for the VIS-MV variant's `MotionCam` UBO + `motion_vec`
/// STORAGE image (the SDF camera-only motion vector) — BYTE-NEUTRAL by the same argument: only the
/// 24-binding VIS-MV layout fills the two new tail slots; the software resolve still fills 19, the
/// RESOLVE_INLINE-hwrt resolve still fills 21, and the base VIS/DENOISED set still fills 22. A
/// `debug_assert!` traps an over-count at `create_bind_group_layout`/`create_bind_group`.
const MAX_BIND_GROUP_BINDINGS: usize = 24;

// The bind-group create path keeps its own copy of the cap so a future divergence
// from the agnostic `boyko_rhi::MAX_BIND_GROUP_BINDINGS` (the desc-side cap) breaks
// the build rather than silently truncating an over-count.
const _: () = assert!(
    MAX_BIND_GROUP_BINDINGS == boyko_rhi::MAX_BIND_GROUP_BINDINGS,
    "backend bind-group cap must match the agnostic boyko_rhi::MAX_BIND_GROUP_BINDINGS"
);

/// `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR` (value `1_000_150_000`) — the
/// `VkDescriptorType` a TLAS binding declares (HW-RT rung R2a-4a). Sourced from the agnostic
/// [`DescriptorKind::AccelerationStructure`] discriminant (its own value-guard pins the value),
/// so the histogram + the write's `descriptor_type` share one source of truth.
const VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR: i32 =
    DescriptorKind::AccelerationStructure.as_i32();

/// The [`DescriptorKind`] slots, in a fixed order, used to bucket a bind group's descriptors
/// into a per-kind histogram for exact pool sizing (Render P1a; the AS slot 5 added at HW-RT
/// rung R2a-4a). [`DESCRIPTOR_KIND_VK`] maps each slot to its `VkDescriptorType`; the two
/// arrays share the slot order. The array length MUST equal [`KIND_COUNT`] — the const-guard
/// below pins it (a slot-count divergence would over/under-run the `create_bind_group`
/// histogram).
const DESCRIPTOR_KIND_VK: [i32; 6] = [
    VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
    VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE,
    VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
    VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
    VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
    VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR,
];

// The histogram's slot count (see `create_bind_group`'s `KIND_COUNT`) MUST equal the
// per-slot `VkDescriptorType` table length — otherwise `descriptor_kind_slot` could return a
// slot the `hist`/`DESCRIPTOR_KIND_VK` arrays cannot index (a release OOB). Pin it here.
const _: () = assert!(DESCRIPTOR_KIND_VK.len() == KIND_COUNT);

// Value-guard: slot 5 is the AS descriptor type (`1_000_150_000`). A wrong value silently
// device-losts (the R2a-1 RT-value lesson — `abi_guard` pins layout, not values).
const _: () = assert!(DESCRIPTOR_KIND_VK[5] == 1_000_150_000);

/// The number of [`DescriptorKind`] histogram slots (== [`DESCRIPTOR_KIND_VK`] length).
/// `create_bind_group` sizes its per-kind `hist`/`pool_sizes` inline arrays to this. Raised
/// 5 → 6 at HW-RT rung R2a-4a for the acceleration-structure slot.
const KIND_COUNT: usize = 6;

/// Maps a [`DescriptorKind`] to its histogram slot in [`DESCRIPTOR_KIND_VK`] (an exhaustive
/// match with NO wildcard, so a new kind fails to compile until it is slotted here).
#[inline]
fn descriptor_kind_slot(kind: DescriptorKind) -> usize {
    match kind {
        DescriptorKind::CombinedImageSampler => 0,
        DescriptorKind::SampledImage => 1,
        DescriptorKind::StorageImage => 2,
        DescriptorKind::UniformBuffer => 3,
        DescriptorKind::StorageBuffer => 4,
        DescriptorKind::AccelerationStructure => 5,
    }
}

/// The [`DescriptorKind`] a [`BindGroupEntry`] variant carries (Render P1a). The
/// per-entry write's `descriptor_type` and the pool histogram both read this, so a
/// new variant must be handled here (exhaustive match, no wildcard).
#[inline]
fn bind_group_entry_kind(entry: &BindGroupEntry<Vulkan>) -> DescriptorKind {
    match entry {
        BindGroupEntry::StorageImage { .. } => DescriptorKind::StorageImage,
        // VG R3 step S1: an explicit view is the SAME descriptor kind as the implicit
        // one — Vulkan has a single `VK_DESCRIPTOR_TYPE_STORAGE_IMAGE`, and only the
        // `VkImageView` handle the write names differs. So this shares the histogram
        // slot, the pool sizing, and the write's `descriptor_type` with `StorageImage`.
        BindGroupEntry::StorageImageView { .. } => DescriptorKind::StorageImage,
        BindGroupEntry::SampledImage { .. } => DescriptorKind::SampledImage,
        // VG R3 step P3-1: a `GENERAL`-layout sampled image is the SAME descriptor kind as a
        // `SHADER_READ_ONLY_OPTIMAL` one — Vulkan has a single
        // `VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE`, and only the layout the write RECORDS differs. So
        // this shares the histogram slot, the pool sizing and the write's `descriptor_type` with
        // `SampledImage`, exactly as `StorageImageView` shares them with `StorageImage`.
        BindGroupEntry::SampledImageAtGeneral { .. } => DescriptorKind::SampledImage,
        BindGroupEntry::CombinedImage { .. } => DescriptorKind::CombinedImageSampler,
        BindGroupEntry::StorageBuffer { .. } => DescriptorKind::StorageBuffer,
        BindGroupEntry::UniformBuffer { .. } => DescriptorKind::UniformBuffer,
        // HW-RT rung R2a-4a: a TLAS binding. The variant is ungated in `boyko_rhi` (its
        // `A::AccelerationStructure` is `()` without `hwrt`), so this arm compiles in both
        // builds; only the `create_bind_group` WRITE branch (which names AS FFI types) is
        // `#[cfg(feature = "hwrt")]`.
        BindGroupEntry::AccelerationStructure { .. } => DescriptorKind::AccelerationStructure,
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
/// range is sized to the LARGEST consumer and every smaller-push pipeline binds
/// against it unchanged. Derived from the consumer constants, never a magic
/// literal, so a future widening of a consumer block re-sizes the range
/// automatically. The value stays within the Vulkan-guaranteed 128-byte floor for
/// `maxPushConstantsSize` (asserted below), so no device-limit query is required.
///
/// VG rung R2c took the "largest consumer" title off the marcher: the batch cull's
/// 104-byte block (six `float4` frustum planes plus two counts) exceeds the
/// marcher's 80. So the derivation is now an explicit `max` over BOTH consumers
/// rather than a single name — which is what this doc always described, and what
/// keeps the next consumer from having to notice which one currently wins.
const COMPUTE_PUSH_CONSTANT_RANGE_BYTES: u32 = {
    let marcher = crate::compute::COMPOSITE_PUSH_CONSTANT_BYTES;
    let batch_cull = crate::compute::VB_BATCH_CULL_PUSH_BYTES;
    if batch_cull > marcher { batch_cull } else { marcher }
};

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
    type QueryPool = VulkanQueryPool;

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
    // VG R3 step S1: the explicit per-mip / per-layer / format-reinterpreting view now
    // that `create_texture_view` is implemented. Nothing binds one yet — the step adds
    // the capability and no owner.
    type TextureView = VulkanTextureView;
    // `Sampler`/`BindGroupLayout`/`BindGroup` bind to the S0 rung-5 concrete types
    // now that `create_sampler`/`create_bind_group_layout`/`create_bind_group` are
    // implemented (the combined-image-sampler graphics descriptor surface).
    type Sampler = VulkanSampler;
    // `GraphicsPipeline` binds to the S0 rung-2 [`VulkanGraphicsPipeline`] now that
    // `create_graphics_pipeline` is implemented.
    type GraphicsPipeline = VulkanGraphicsPipeline;
    type BindGroup = VulkanBindGroup;
    type BindGroupLayout = VulkanBindGroupLayout;
    // HW-RT rung R1: the cheapest placeholder — no `VkAccelerationStructureKHR`
    // FFI, no verbs, no RT extension. R2a-1 rebinds this to the concrete
    // `BoundAccelStruct` under `feature="hwrt"`; a default build keeps `()` (byte-identical,
    // and the AS verbs stay the RhiDevice/RhiCommandEncoder `#[cold]` erroring defaults).
    #[cfg(not(feature = "hwrt"))]
    type AccelerationStructure = ();
    #[cfg(feature = "hwrt")]
    type AccelerationStructure = crate::accel::BoundAccelStruct;
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
/// `layout` is the target a [`RhiCommandEncoder::bind_descriptor_set_compute`](boyko_rhi::RhiCommandEncoder::bind_descriptor_set_compute) binds
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
/// also the target of [`RhiCommandEncoder::push_graphics_constants`](boyko_rhi::RhiCommandEncoder::push_graphics_constants). Its shader
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

impl VulkanBindGroupLayout {
    /// Multi-paradigm render-path plan, rung R4b-b: the raw `VkDescriptorSetLayout` handle —
    /// a public accessor for cross-crate callers that need to hand a layout to a MULTI-SET
    /// pipeline builder taking raw handles (e.g.
    /// [`VulkanContext::create_graphics_pipeline_forward`]'s `set1_placeholder`/`set2_layout`
    /// parameters), mirroring [`crate::bindless::VulkanBindlessSet::set_layout`]'s existing
    /// public-accessor precedent for the SAME `create_graphics_pipeline_bindless` shape.
    #[inline]
    pub fn set_layout(&self) -> VkDescriptorSetLayout {
        self.set_layout
    }
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

/// Rewrites binding `binding` of `bg`'s descriptor set in place to point at `buffer` — a
/// single `vkUpdateDescriptorSets` storage-buffer write, reusing the SAME device fn pointer
/// [`VulkanCommandEncoder::bind_storage_buffer`](boyko_rhi::RhiCommandEncoder::bind_storage_buffer)'s one-time compute-set write uses
/// (`update_descriptor_sets`). Asset-streaming plan F7 §5: growing a GPU-mirrored SSBO
/// repoints every descriptor set that binds it in place, without a `vkDeviceWaitIdle` —
/// the surgical tool the present-crate rebind orchestration (`GBufferFrame::
/// repoint_material_table`) and the host's per-slot instance-family growth both drive.
///
/// # Safety
///
/// The caller guarantees `bg`'s descriptor set is not bound to any command buffer currently
/// pending execution (VUID-vkUpdateDescriptorSets-None-03047) — i.e. every submit that could
/// reference it has already been fence-waited (the fenced-slot discipline every F7 caller
/// relies on). `ctx` must be the live context `bg` and `buffer` were created on.
pub unsafe fn rebind_storage_buffer(
    ctx: &VulkanContext,
    bg: &VulkanBindGroup,
    binding: u32,
    buffer: &BoundBuffer,
) {
    let buffer_info = VkDescriptorBufferInfo {
        buffer: buffer.buffer,
        offset: 0,
        range: buffer.size,
    };
    let write = VkWriteDescriptorSet {
        s_type: VkStructureType::WriteDescriptorSet,
        p_next: ptr::null(),
        dst_set: bg.descriptor_set,
        dst_binding: binding,
        dst_array_element: 0,
        descriptor_count: 1,
        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        p_image_info: ptr::null(),
        p_buffer_info: &buffer_info,
        p_texel_buffer_view: ptr::null(),
    };
    let fns = ctx.device_fns();
    // SAFETY: `ctx.device()`/`ctx.device_fns()` are the live device + its command table
    // (`ctx` is live per the caller contract above); the write references the live
    // `buffer_info` local + `bg`'s live descriptor set; `bg`'s set is not command-buffer-
    // pending (caller contract above), so updating it in place is sound.
    unsafe { (fns.update_descriptor_sets)(ctx.device(), 1, &write, 0, ptr::null()) };
}

/// Rewrites binding `binding` of `bg`'s descriptor set in place to point at `accel` — the
/// HW-RT acceleration-structure counterpart of [`rebind_storage_buffer`]: a single
/// `vkUpdateDescriptorSets` write through the SAME `VkWriteDescriptorSetAccelerationStructureKHR`
/// `p_next` chain [`crate::rhi_impl::device::create_bind_group`]'s
/// `BindGroupEntry::AccelerationStructure` arm uses (HW-RT rung R2a-4a). Asset-streaming
/// plan F7-hwrt (task#11): growing the per-slot TLAS mints a NEW `VkAccelerationStructureKHR`
/// handle — every resolve-family descriptor set that traces it must be repointed here, or
/// it dangles at the freed handle the instant the old TLAS is retired.
///
/// # Safety
///
/// The caller guarantees `bg`'s descriptor set is not bound to any command buffer currently
/// pending execution (VUID-vkUpdateDescriptorSets-None-03047) — the same fenced-slot
/// discipline [`rebind_storage_buffer`] relies on. `ctx` must be the live context `bg` and
/// `accel` were created on; `accel` must outlive every submit that could reference it.
#[cfg(feature = "hwrt")]
pub unsafe fn rebind_accel_struct(
    ctx: &VulkanContext,
    bg: &VulkanBindGroup,
    binding: u32,
    accel: &BoundAccelStruct,
) {
    let as_write = VkWriteDescriptorSetAccelerationStructureKHR {
        s_type: ST_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
        _pad: 0,
        p_next: ptr::null(),
        acceleration_structure_count: 1,
        _pad2: 0,
        // `accel.handle` lives in the caller's live `&BoundAccelStruct` borrow (address
        // stable for this call); taking its address does not copy the handle into a local.
        p_acceleration_structures: &accel.handle,
    };
    let write = VkWriteDescriptorSet {
        s_type: VkStructureType::WriteDescriptorSet,
        p_next: (&as_write as *const VkWriteDescriptorSetAccelerationStructureKHR).cast::<c_void>(),
        dst_set: bg.descriptor_set,
        dst_binding: binding,
        dst_array_element: 0,
        descriptor_count: 1,
        descriptor_type: VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR,
        p_image_info: ptr::null(),
        p_buffer_info: ptr::null(),
        p_texel_buffer_view: ptr::null(),
    };
    let fns = ctx.device_fns();
    // SAFETY: `ctx.device()`/`ctx.device_fns()` are the live device + its command table
    // (`ctx` is live per the caller contract above); the write's `p_next` points at the
    // live `as_write` local (alive for the whole call), whose `p_acceleration_structures`
    // points at `accel.handle` inside the caller's live `&BoundAccelStruct` borrow (also
    // alive for the whole call); `bg`'s set is not command-buffer-pending (caller contract
    // above), so updating it in place is sound.
    unsafe { (fns.update_descriptor_sets)(ctx.device(), 1, &write, 0, ptr::null()) };
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

/// An owned GPU timestamp-query pool ([`RhiApi::QueryPool`], HW-RT rung R0).
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this pool is read from
/// or destroyed: each goes through the context's device fn-table. Its queries are
/// UNDEFINED until reset ([`RhiCommandEncoder::reset_query_pool`](boyko_rhi::RhiCommandEncoder::reset_query_pool)) each frame before
/// the first [`RhiCommandEncoder::write_timestamp`](boyko_rhi::RhiCommandEncoder::write_timestamp). No compile-time `'ctx` tie this
/// phase (plan F1; the fence precedent).
pub struct VulkanQueryPool {
    /// The `VkQueryPool` handle; destroyed by `destroy_query_pool`.
    pub(crate) pool: VkQueryPool,
    /// The number of queries the pool holds (`2 * PASS_COUNT` for the bracket
    /// collector); used to `debug_assert` a read stays in bounds.
    pub(crate) count: u32,
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

/// The hot command-recording encoder ([`RhiCommandEncoder`](boyko_rhi::RhiCommandEncoder)).
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
    /// HW-RT rung R2a-1: a raw pointer into the owning context's `AccelFns` (the AS command
    /// table), or `null` when ray query is off. Set by `create_command_encoder` under `hwrt`
    /// (mirroring the `*const DeviceFns` discipline — the context outlives the encoder). Gated
    /// `hwrt`: absent from a default build (the encoder layout is textually R1 there).
    #[cfg(feature = "hwrt")]
    accel_fns: *const crate::accel::AccelFns,
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

mod device;
mod encoder;
