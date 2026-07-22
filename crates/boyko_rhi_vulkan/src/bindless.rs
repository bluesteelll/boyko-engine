//! T4 — the bindless texture-array descriptor set: a runtime-sized
//! `Texture2D gTextures[]` array (set 1 of the TEXTURED raster pipeline, bound at
//! textured-PBR T6c) plus its paired shared trilinear + anisotropic sampler.
//!
//! # Layout shape (decision)
//!
//! One dedicated 2-binding `VkDescriptorSetLayout`, distinct from the generic
//! Render-P1a [`crate::rhi_impl::VulkanBindGroupLayout`] path (which has no seam
//! for `VkDescriptorSetLayoutBindingFlagsCreateInfo` / UPDATE_AFTER_BIND pool bits
//! / a variable-count allocation — every P1a set is written exactly once at create
//! and never touched again):
//!
//! * **binding 0** — `SAMPLED_IMAGE`, `descriptorCount = `[`BINDLESS_TEXTURE_CAPACITY`],
//!   flagged `PARTIALLY_BOUND | UPDATE_AFTER_BIND` (NOT `VARIABLE_DESCRIPTOR_COUNT` —
//!   spec-legal only on the LAST binding, and binding 1 sits after this one; the set
//!   is allocated at the full declared capacity instead). The
//!   HLSL-visible `Texture2D gTextures[] : register(t0, space1)`.
//! * **binding 1** — `SAMPLER`, `descriptorCount = 1`, declared with an IMMUTABLE
//!   sampler (`pImmutableSamplers` points at the one shared `VkSampler` baked in at
//!   layout-create time). The HLSL-visible `SamplerState gSampler : register(s0,
//!   space1)`. Because it is immutable, it is NEVER written via
//!   `vkUpdateDescriptorSets` — [`write_bindless_texture`] only ever targets
//!   binding 0.
//!
//! A single shared sampler (not a per-texture combined-image-sampler) is the
//! common HLSL-friendly split for a bindless table: every texture reuses the same
//! filtering, so there is exactly one sampler to manage, and binding 0 stays a
//! pure `SAMPLED_IMAGE` array (a plain index, no per-slot sampler bookkeeping).
//!
//! # Device-UAF safety (no validation layer on this box — see crate docs)
//!
//! Three STRUCTURAL guards make an invalid read UB-free rather than
//! runtime-checked:
//!
//! 1. **Bounds** — every write and every allocator-issued slot is `< capacity`
//!    (`debug_assert!` here; the allocator never issues an out-of-range slot by
//!    construction — see `boyko_render::bindless::BindlessSlotAllocator`).
//! 2. **Error texture in every slot** — [`crate::bindless`]'s caller
//!    (`boyko_render::bindless::BindlessTextureTable::new`) writes the magenta
//!    error texture into EVERY slot (including the reserved slot 0) before any
//!    real texture is registered, so an unwritten/stale index samples a visibly
//!    wrong texture, never UNDEFINED/garbage memory.
//! 3. **Fence-gated slot recycle (P1-5)** — a freed slot is NOT returned to the
//!    allocator's free list until the fence horizon that could still reference it
//!    has passed (mirrors the F6/F7 `RetiredGpuBuffers` discipline). This module
//!    provides the mechanism (UPDATE_AFTER_BIND lets a write land on a LIVE set
//!    with no rebuild); the recycle POLICY lives in `boyko_render::bindless`
//!    (device-free, unit-testable).

use core::ffi::c_void;
use core::ptr;

use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::ffi::*;

/// The bindless texture array's declared capacity (binding 0's `descriptorCount`).
///
/// Picked well under the Vulkan 1.2 core "Required Limits" floor guaranteed for
/// any device reporting the descriptor-indexing feature bits this engine's boot
/// path already fail-fasts on (`DeviceCaps::bindless_capable` —
/// `shaderSampledImageArrayNonUniformIndexing` + `runtimeDescriptorArray` +
/// `descriptorBindingPartiallyBound` + `descriptorBindingVariableDescriptorCount` +
/// `descriptorBindingSampledImageUpdateAfterBind`): the spec's required-limits
/// table guarantees `maxDescriptorSetUpdateAfterBindSampledImages >= 500,000` for
/// such a device, so no runtime property query is needed to justify this constant
/// — mirrors `rhi_impl::COMPUTE_PUSH_CONSTANT_RANGE_BYTES`'s
/// `VULKAN_MIN_MAX_PUSH_CONSTANTS_SIZE` precedent (stay comfortably within a
/// documented guaranteed floor rather than probe a property at boot). 4096 is
/// generous headroom over any texture budget this engine's scenes need; slot 0 is
/// reserved (see [`crate::bindless`] module docs), leaving 4095 real slots.
pub const BINDLESS_TEXTURE_CAPACITY: u32 = 4096;

/// The bindless texture array's binding index (`register(t0, space1)`).
pub const BINDLESS_IMAGE_BINDING: u32 = 0;

/// The shared sampler's binding index (`register(s0, space1)`) — declared with an
/// IMMUTABLE sampler, never written at runtime.
pub const BINDLESS_SAMPLER_BINDING: u32 = 1;

/// The owned bindless descriptor set: its dedicated 2-binding layout, its
/// UPDATE_AFTER_BIND pool, the allocated set, and the shared immutable sampler
/// baked into binding 1.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this set is written
/// ([`write_bindless_texture`]), bound, or destroyed ([`destroy_bindless_texture_set`]):
/// each goes through the context's device fn-table. No compile-time `'ctx` tie
/// this phase (mirrors every other RHI resource, plan F1).
pub struct VulkanBindlessSet {
    pub(crate) set_layout: VkDescriptorSetLayout,
    pub(crate) pool: VkDescriptorPool,
    pub(crate) set: VkDescriptorSet,
    /// The immutable shared sampler baked into binding 1 at layout-creation time.
    /// Vulkan requires an immutable sampler referenced by a layout's
    /// `pImmutableSamplers` to remain valid for the layout's whole lifetime, so it
    /// is destroyed AFTER `set_layout` in [`destroy_bindless_texture_set`].
    pub(crate) sampler: VkSampler,
    capacity: u32,
}

impl VulkanBindlessSet {
    /// The runtime array's declared capacity (binding 0's `descriptorCount`) —
    /// every valid slot satisfies `slot < capacity()`.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// The raw `VkDescriptorSet` — bound directly at set 1 by the textured gbuffer
    /// raster pass's `cmd_bind_descriptor_sets` (textured-PBR T6c).
    #[inline]
    pub fn set(&self) -> VkDescriptorSet {
        self.set
    }

    /// The raw `VkDescriptorSetLayout` — passed directly as
    /// [`VulkanContext::create_graphics_pipeline_bindless`](crate::device::VulkanContext::create_graphics_pipeline_bindless)'s
    /// `set1_layout` argument at TEXTURED raster pipeline creation (textured-PBR T6c).
    #[inline]
    pub fn set_layout(&self) -> VkDescriptorSetLayout {
        self.set_layout
    }
}

/// Builds the shared immutable sampler: trilinear (`LINEAR` mag/min/mip) +
/// anisotropic (capped to the Vulkan-1.2 guaranteed `maxSamplerAnisotropy >= 16`
/// floor for a `samplerAnisotropy`-capable device — T-dev already enables that
/// feature) + repeat addressing (the common wrap mode for a tiled material
/// texture) + no compare + unclamped mip range.
///
/// Deliberately bypasses [`crate::device::VulkanContext::device_caps`]'s
/// `create_sampler`/`SamplerDesc` seam: `SamplerDesc` has no anisotropy field and
/// its `MipMode` has no trilinear variant today (every other sampler caller is
/// single-mip nearest/bilinear), and widening that public, ~20-call-site struct
/// is out of this rung's scope — this is a dedicated, additive, raw-FFI
/// construction local to the bindless path.
fn create_shared_sampler(ctx: &VulkanContext) -> Result<VkSampler, VulkanError> {
    // The Vulkan 1.2 core "Required Limits" table guarantees `maxSamplerAnisotropy
    // >= 16.0` for any device with the `samplerAnisotropy` feature enabled (T-dev
    // already requests it — `sampler_anisotropy: VK_TRUE` at device-create) — no
    // runtime limit query needed, mirroring this module's `BINDLESS_TEXTURE_CAPACITY`
    // reasoning.
    const MAX_ANISOTROPY: f32 = 16.0;
    // A sufficiently large max LOD that no realistic mip chain clamps against
    // (mirrors the common `VK_LOD_CLAMP_NONE` idiom); a single-mip source image
    // (no mipmaps generated yet) always resolves to level 0 regardless, since the
    // image itself has only one level.
    const MAX_LOD_UNCLAMPED: f32 = 1000.0;

    let info = VkSamplerCreateInfo {
        s_type: VkStructureType::SamplerCreateInfo,
        p_next: ptr::null(),
        flags: 0,
        mag_filter: VK_FILTER_LINEAR,
        min_filter: VK_FILTER_LINEAR,
        mipmap_mode: VK_SAMPLER_MIPMAP_MODE_LINEAR,
        address_mode_u: VK_SAMPLER_ADDRESS_MODE_REPEAT,
        address_mode_v: VK_SAMPLER_ADDRESS_MODE_REPEAT,
        address_mode_w: VK_SAMPLER_ADDRESS_MODE_REPEAT,
        mip_lod_bias: 0.0,
        anisotropy_enable: VK_TRUE,
        max_anisotropy: MAX_ANISOTROPY,
        compare_enable: VK_FALSE,
        compare_op: VK_COMPARE_OP_NEVER,
        min_lod: 0.0,
        max_lod: MAX_LOD_UNCLAMPED,
        border_color: VK_BORDER_COLOR_FLOAT_OPAQUE_BLACK,
        unnormalized_coordinates: VK_FALSE,
    };
    let mut sampler = VkSampler::NULL;
    // SAFETY: `ctx.device()` is live; `info` is a fully-initialized `#[repr(C)]`
    // `VkSamplerCreateInfo` (null `p_next`, no GPU memory backing a sampler);
    // `&mut sampler` is a valid out-pointer; NULL allocator.
    let raw = unsafe {
        (ctx.device_fns().create_sampler)(ctx.device(), &info, ptr::null(), &mut sampler)
    };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(VulkanError::Vk("vkCreateSampler(bindless)", result));
    }
    Ok(sampler)
}

/// Creates the bindless texture-array descriptor set (T4): the 2-binding
/// UPDATE_AFTER_BIND-pool layout (binding 0 = the `SAMPLED_IMAGE` runtime array,
/// binding 1 = the immutable shared sampler), an UPDATE_AFTER_BIND-flagged pool
/// sized for exactly one set, and the set itself allocated at the layout's
/// declared full [`BINDLESS_TEXTURE_CAPACITY`] (plain fixed-count allocation —
/// `VARIABLE_DESCRIPTOR_COUNT` is spec-legal only on the LAST binding, and the
/// sampler binding sits after the array).
///
/// No slot is written here — every slot (including the reserved slot 0) starts as
/// an uninitialized `SAMPLED_IMAGE` descriptor; the caller
/// (`boyko_render::bindless::BindlessTextureTable::new`) writes the error texture
/// into every slot immediately after, per this module's device-UAF-safety guard
/// #2. On any partial failure every object created so far is torn down before the
/// error returns (no leak).
pub fn create_bindless_texture_set(ctx: &VulkanContext) -> Result<VulkanBindlessSet, VulkanError> {
    debug_assert!(
        ctx.device_caps().bindless_capable,
        "invariant: a booted VulkanContext always satisfies bindless_capable (boot fail-fast)"
    );

    let sampler = create_shared_sampler(ctx)?;
    let device = ctx.device();
    let fns = ctx.device_fns();

    // Both bindings are visible to FRAGMENT (the Deferred `gbuffer_mrt.fs` TEXTURED consumer)
    // AND COMPUTE: rung TV0's `vb_shade_tex.comp` reuses this SAME bindless layout OBJECT (R5)
    // as its Set 3. Without `COMPUTE_BIT` the compute stage is not permitted to touch
    // `gTextures[]`/`gTexSampler`, so every `SampleGrad` silently returns 0 on a validation-off
    // device — the TV0 all-black-textured-sphere bug. Widening only ADDS a stage: the Deferred
    // fragment path (and its goldens) is byte-unaffected.
    let bindings = [
        VkDescriptorSetLayoutBinding {
            binding: BINDLESS_IMAGE_BINDING,
            descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE,
            descriptor_count: BINDLESS_TEXTURE_CAPACITY,
            stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT | VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        },
        VkDescriptorSetLayoutBinding {
            binding: BINDLESS_SAMPLER_BINDING,
            descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLER,
            descriptor_count: 1,
            stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT | VK_SHADER_STAGE_COMPUTE_BIT,
            // Immutable: baked into the layout, never written via
            // `vkUpdateDescriptorSets` — `&sampler` outlives this call (a local
            // alive for the whole function).
            p_immutable_samplers: (&sampler as *const VkSampler).cast(),
        },
    ];
    // Binding 0 is bindless; binding 1 (the immutable sampler) needs none of the
    // flags — a `0` entry is valid (an immutable sampler is never updated).
    // NO `VARIABLE_DESCRIPTOR_COUNT` (validation-audit fix): the spec allows that
    // flag ONLY on the layout's LAST binding, and binding 1 (the sampler) sits
    // after the array. It bought nothing anyway — the allocation always supplied
    // the FULL `BINDLESS_TEXTURE_CAPACITY` as the variable count, which is exactly
    // what a plain fixed-count allocation of this layout yields.
    let binding_flags: [VkFlags; 2] = [
        VK_DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT | VK_DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT,
        0,
    ];
    let binding_flags_info = VkDescriptorSetLayoutBindingFlagsCreateInfo {
        s_type: VkStructureType::DescriptorSetLayoutBindingFlagsCreateInfo,
        p_next: ptr::null(),
        binding_count: binding_flags.len() as u32,
        p_binding_flags: binding_flags.as_ptr(),
    };
    let layout_info = VkDescriptorSetLayoutCreateInfo {
        s_type: VkStructureType::DescriptorSetLayoutCreateInfo,
        p_next: (&binding_flags_info as *const VkDescriptorSetLayoutBindingFlagsCreateInfo)
            .cast::<c_void>(),
        flags: VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT,
        binding_count: bindings.len() as u32,
        p_bindings: bindings.as_ptr(),
    };
    let mut set_layout = VkDescriptorSetLayout::NULL;
    // SAFETY: `device` is live; `layout_info` is fully initialized, its
    // `p_bindings` points at the live `bindings` array (2 entries, alive for this
    // call) and its `p_next` chains `binding_flags_info` (alive for this call),
    // whose `p_binding_flags` points at the live `binding_flags` array (2 entries,
    // same count as `bindings` — the driver reads exactly `binding_count` of
    // each); binding 1's `p_immutable_samplers` points at the live `sampler`
    // local; `&mut set_layout` is a valid out-pointer; NULL allocator.
    let raw = unsafe {
        (fns.create_descriptor_set_layout)(device, &layout_info, ptr::null(), &mut set_layout)
    };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        // SAFETY: `sampler` was just created on `device`, owned exclusively here,
        // never bound to any set; destroy it once on this edge.
        unsafe { (fns.destroy_sampler)(device, sampler, ptr::null()) };
        return Err(VulkanError::Vk(
            "vkCreateDescriptorSetLayout(bindless)",
            result,
        ));
    }

    let pool_sizes = [
        VkDescriptorPoolSize {
            descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE,
            descriptor_count: BINDLESS_TEXTURE_CAPACITY,
        },
        VkDescriptorPoolSize {
            descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLER,
            descriptor_count: 1,
        },
    ];
    let pool_info = VkDescriptorPoolCreateInfo {
        s_type: VkStructureType::DescriptorPoolCreateInfo,
        p_next: ptr::null(),
        flags: VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT,
        max_sets: 1,
        pool_size_count: pool_sizes.len() as u32,
        p_pool_sizes: pool_sizes.as_ptr(),
    };
    let mut pool = VkDescriptorPool::NULL;
    // SAFETY: `device` is live; `pool_info` is fully initialized referencing the
    // live `pool_sizes` array (2 entries, alive for this call); `&mut pool` is a
    // valid out-pointer; NULL allocator.
    let raw = unsafe { (fns.create_descriptor_pool)(device, &pool_info, ptr::null(), &mut pool) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        // SAFETY: `set_layout`/`sampler` were just created on `device`, owned
        // exclusively here, never bound to any command buffer; destroy each once,
        // in reverse order, on this edge.
        unsafe {
            (fns.destroy_descriptor_set_layout)(device, set_layout, ptr::null());
            (fns.destroy_sampler)(device, sampler, ptr::null());
        }
        return Err(VulkanError::Vk(
            "vkCreateDescriptorPool(bindless)",
            result,
        ));
    }

    // Plain fixed-count allocation (validation-audit fix): the layout no longer
    // carries `VARIABLE_DESCRIPTOR_COUNT` (spec: last-binding-only, and binding 1
    // sits after the array), so no `VkDescriptorSetVariableDescriptorCountAllocateInfo`
    // chain — the set is allocated at the layout's declared full
    // `BINDLESS_TEXTURE_CAPACITY`, byte-for-byte what the removed chain supplied.
    let alloc_info = VkDescriptorSetAllocateInfo {
        s_type: VkStructureType::DescriptorSetAllocateInfo,
        p_next: ptr::null(),
        descriptor_pool: pool,
        descriptor_set_count: 1,
        p_set_layouts: &set_layout,
    };
    let mut set = VkDescriptorSet::NULL;
    // SAFETY: `device` is live; `alloc_info` names the live `pool` + the live
    // `set_layout` local; `&mut set` is a valid out-pointer for the single set.
    let raw = unsafe { (fns.allocate_descriptor_sets)(device, &alloc_info, &mut set) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        // SAFETY: `pool` owns no live set yet (allocation failed); `set_layout`/
        // `sampler` are owned exclusively here; destroy each once, in reverse
        // order, on this edge (destroying the pool also frees any
        // partially-allocated set).
        unsafe {
            (fns.destroy_descriptor_pool)(device, pool, ptr::null());
            (fns.destroy_descriptor_set_layout)(device, set_layout, ptr::null());
            (fns.destroy_sampler)(device, sampler, ptr::null());
        }
        return Err(VulkanError::Vk(
            "vkAllocateDescriptorSets(bindless)",
            result,
        ));
    }

    Ok(VulkanBindlessSet {
        set_layout,
        pool,
        set,
        sampler,
        capacity: BINDLESS_TEXTURE_CAPACITY,
    })
}

/// Destroys `s`, consuming it: the pool (which implicitly frees the allocated
/// set), then the layout, then the sampler LAST (Vulkan requires an immutable
/// sampler referenced by a layout to outlive that layout).
///
/// # Safety
///
/// `ctx` must be the live context `s` was created on; no submission using `s.set`
/// is pending (fence-waited / `wait_idle`'d); `s` is destroyed exactly once (the
/// by-value move enforces this).
pub unsafe fn destroy_bindless_texture_set(ctx: &VulkanContext, s: VulkanBindlessSet) {
    let device = ctx.device();
    let fns = ctx.device_fns();
    // SAFETY: per this fn's contract `device` is live and no submission references
    // `s`; each object is destroyed exactly once, in reverse creation order
    // (pool → layout → sampler — the sampler last, since the layout's immutable
    // binding referenced it).
    unsafe {
        (fns.destroy_descriptor_pool)(device, s.pool, ptr::null());
        (fns.destroy_descriptor_set_layout)(device, s.set_layout, ptr::null());
        (fns.destroy_sampler)(device, s.sampler, ptr::null());
    }
}

/// Writes ONE texture into `set`'s bindless array at `slot` — a single
/// `vkUpdateDescriptorSets` `SAMPLED_IMAGE` write with `dstArrayElement = slot`,
/// `descriptorCount = 1`.
///
/// Distinct from [`crate::rhi_impl::VulkanBindGroup`]'s create-time batched write
/// (which writes every binding of a set exactly ONCE at `create_bind_group`): a
/// bindless set is written incrementally, one slot at a time, at ASSET LOAD TIME
/// — `set`'s layout was created with the UPDATE_AFTER_BIND pool/layout bits (see
/// [`create_bindless_texture_set`]), so this write is valid even while OTHER
/// slots of the SAME live set are bound to an in-flight command buffer
/// (`VUID-vkUpdateDescriptorSets-None-03047` is scoped to non-UPDATE_AFTER_BIND
/// bindings).
///
/// No `sampler` parameter: binding 0 is a pure `SAMPLED_IMAGE` (the shared sampler is
/// IMMUTABLE at binding 1, baked in at [`create_bindless_texture_set`] time and never
/// written here — see the module docs), so a per-write sampler would be silently
/// ignored by the driver for this descriptor type; removed (YAGNI) rather than kept
/// for a hypothetical future non-bindless write — every caller passed `None`.
///
/// # Safety
///
/// The caller guarantees:
/// * `slot < set.capacity()` — an out-of-range `dstArrayElement` is an OOB
///   descriptor-set write.
/// * `image_view` is a live `VkImageView` in `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL`
///   for as long as any shader invocation may index this slot — including every
///   in-flight frame at the moment of the write, since UPDATE_AFTER_BIND permits
///   OTHER slots to be read concurrently with this write but the driver may
///   schedule the write to apply to a NOT-YET-DISPATCHED read of THIS same slot;
///   the caller's fence-gated slot-recycle discipline (`BindlessSlotAllocator`,
///   `boyko_render::bindless`) is what guarantees no ALREADY-IN-FLIGHT shader
///   invocation is still indexing `slot` when this write targets a REUSED slot —
///   a freshly-allocated (never-before-issued) slot has no prior in-flight
///   reference by construction.
/// * `ctx` is the live context `set` was created on.
pub unsafe fn write_bindless_texture(
    ctx: &VulkanContext,
    set: &VulkanBindlessSet,
    binding: u32,
    slot: u32,
    image_view: VkImageView,
) {
    debug_assert!(
        slot < set.capacity,
        "invariant: bindless slot must be < capacity"
    );
    let image_info = VkDescriptorImageInfo {
        sampler: VkSampler::NULL,
        image_view,
        image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
    };
    let write = VkWriteDescriptorSet {
        s_type: VkStructureType::WriteDescriptorSet,
        p_next: ptr::null(),
        dst_set: set.set,
        dst_binding: binding,
        dst_array_element: slot,
        descriptor_count: 1,
        descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE,
        p_image_info: (&image_info as *const VkDescriptorImageInfo).cast::<c_void>(),
        p_buffer_info: ptr::null(),
        p_texel_buffer_view: ptr::null(),
    };
    let fns = ctx.device_fns();
    // SAFETY: `ctx.device()`/`fns` are the live device + its command table (`ctx`
    // is live per the caller contract above); the write references the live
    // `image_info` local (alive for this call) naming the caller's live
    // `image_view` at `slot < set.capacity` (checked above); `set.set` was
    // allocated from an UPDATE_AFTER_BIND-flagged pool against an
    // UPDATE_AFTER_BIND-flagged layout, so writing it while other slots may be
    // bound to in-flight work is valid; the caller's fence-gated recycle
    // discipline (this fn's `# Safety`) guarantees no in-flight read of THIS slot
    // races this specific write.
    unsafe { (fns.update_descriptor_sets)(ctx.device(), 1, &write, 0, ptr::null()) };
}
